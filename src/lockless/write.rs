use crate::lockless::read::{Reader, ReaderEpochs};
use crate::lockless::table::{Table, TableWriteGuard};
use crate::types::*;
use slab::Slab;
use std::cell::UnsafeCell;
use std::fmt;

/// InnerWriter is the entry point for using active_standy primitives, and for
/// updating the underlying table. It is responsible for creating the tables,
/// and is how to create the first reader. InnerWriter is responsible for handling
/// the synchronization with Readers, making sure to update them to the new
/// active table when swapped, and making sure not to mutate the standby table
/// if there are any Readers remaining.
///
/// In order to interact with the underlying tables you must create a
/// InnerWriteGuard. Only 1 InnerWriter can exist for a given table.
///
/// InnerWriter doesn't actually own the underlying data, so if InnerWriter is Dropped,
/// this will not delete the tables. Instead they will only be dropped once all
/// Readers and the InnerWriter are dropped.
///
/// For examples of using InnerWriter check out the tests in this file.
pub struct InnerWriter<T> {
    // The underlying tables. This struct is responsible for returning the
    // correct active/standby table, and also for swapping them when the
    // TableInnerWriteGuard is dropped. This table does not handle any
    // synchronization across InnerWriter/Readers, rather that is handled by the
    // InnerWriter and Readers themselves.
    table: Arc<Table<T>>,

    /// Information about each of the readers. Used by the InnerWriter and Readers to
    /// synchronize so that the InnerWriter never mutates a table that a ReadGuard is
    /// pointing to.
    readers: ReaderEpochs,

    /// Log of operations to be performed on the second table. This gets played
    /// on the standby table when creating a InnerWriteGuard, as opposed to when
    /// dropping it, to minimize lock contention. This is in the hopes that by
    /// waiting until the next time a InnerWriteGuard is created, we give the readers
    /// time to switch to reading from the new active_table.
    ops_to_replay: Vec<Box<dyn FnOnce(&mut T) + Send>>,

    // Record the epoch of the readers after we swap the tables. This is used to
    // tell the InnerWriter when it is safe to mutate the standby_table. InnerWriter only
    // mutates this by removing entries when waiting for the standby table to be
    // free. {reader_key : first_epoch_after_swap}.
    blocking_readers: std::collections::HashMap<usize, usize>,
}

/// InnerWriteGuard is the interface used to actually mutate the tables. The
/// lifecycle of a InnerWriteGuard is:
/// 1. Creation - Wait for all Readers to point to the active_table, then apply
///    all updates on the standby_table via 'apply_second'. This consumes all of
///    the udpates.
/// 2. Duration - update the standby table synchronously as updates come in via
///    'apply_first'. Updates are then held onto for the next InnerWriteGuard to
///    apply on the other table.
/// 3. Drop - When a InnerWriteGuard is dropped, swap the active and standby tables,
///    publishing all of the updates to the Readers. This is the only time that
///    the tables can be swapped.
///
/// Only 1 InnerWriteGuard can exist at a time.
pub struct InnerWriteGuard<'w, T> {
    // A wrapper around the underlying tables which allows us to mutate the
    // standby table. On Drop, we will tell the guard to swap the active and
    // standby tables, publishing all updates to the Readers.
    table: TableWriteGuard<'w, T>,

    // Record the ops that were applied to standby_table to be replayed the next
    // time we create a InnerWriteGuard. We hold a FnOnce instead of an UpdateTables,
    // because UpdateTables is templated on the return type, which is only used
    // in apply_first.
    ops_to_replay: &'w mut Vec<Box<dyn FnOnce(&mut T) + Send>>,

    // Used to record first_epoch_after_swap *after* we have swapped the active
    // and standby tables.
    readers: &'w ReaderEpochs,

    // Record the epoch of the readers after we swap the tables. This is used to
    // tell the InnerWriter when it is safe to mutate the standby_table. InnerWriteGuard
    // only mutates this by adding entries after swapping the active and standby
    // tables. {reader_key : first_epoch_after_swap}.
    blocking_readers: &'w mut std::collections::HashMap<usize, usize>,
}

impl<T> InnerWriter<T> {
    /// Create a InnerWriter object for handling active_standby tables.
    /// - t1 & t2 are the two tables which will become the active and standby
    ///   tables. They must be identical; this is left to the user to enforce.
    fn from_identical(t1: T, t2: T) -> InnerWriter<T> {
        InnerWriter {
            table: Arc::new(Table::from_identical(t1, t2)),
            readers: Arc::new(Mutex::new(Slab::with_capacity(1024))),
            ops_to_replay: Vec::new(),
            blocking_readers: std::collections::HashMap::new(),
        }
    }

    // Hangs until the standby table has no readers pointing to it, meaning it
    // is safe for updating.
    fn await_standby_table_free(&mut self) {
        // Wait until no reader is making use of the standby table.
        while !self.blocking_readers.is_empty() {
            {
                let readers = self.readers.lock().unwrap();
                self.blocking_readers.retain(|key, first_epoch_after_swap| {
                    let epoch = match readers.get(*key) {
                        None => {
                            // This Reader has been dropped.
                            return false;
                        }
                        Some(epoch) => epoch.load(Ordering::Acquire),
                    };

                    epoch <= *first_epoch_after_swap && *first_epoch_after_swap % 2 != 0
                });
            }

            if !self.blocking_readers.is_empty() {
                // Instead of busy looping we will yield this thread and come
                // back when the OS returns to us.
                yield_now();
            }
        }
    }

    /// Create a InnerWriteGuard to allow users to update the the data. There will
    /// only be 1 InnerWriteGuard at a time.
    ///
    /// This function may be slow because:
    /// 1. Lock contention on the standby_table. This can occur if a ReadGuard
    ///    which was created before the last InnerWriteGuard was dropped, still has
    ///    not itself been dropped.
    /// 2. Replaying all of the updates that were applied to the last
    ///    InnerWriteGuard.
    pub fn write(&mut self) -> InnerWriteGuard<'_, T> {
        // Wait until the standby table is free for us to update.
        self.await_standby_table_free();

        // Make sure we don't reorder waiting till later. Technically it just
        // has to happen before we replay ops, since grabbing the standby table
        // is fine to do without mutating it since this is the only InnerWriter.
        std::sync::atomic::compiler_fence(Ordering::SeqCst);

        let mut table = self.table.write();

        // Replay all ops on the standby table.
        for op in self.ops_to_replay.drain(..) {
            op(&mut table);
        }
        self.ops_to_replay.clear();

        InnerWriteGuard {
            table,
            ops_to_replay: &mut self.ops_to_replay,
            readers: &self.readers,
            blocking_readers: &mut self.blocking_readers,
        }
    }

    pub fn new_reader(&self) -> Reader<T> {
        Reader::new(&self.readers, &self.table)
    }
}

// This is the Debug string that will be printed out, so don't specify that this
// is InnerWriter.
impl<T: fmt::Debug> fmt::Debug for InnerWriter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("num_ops_to_replay", &self.ops_to_replay.len())
            .finish()
    }
}

impl<'w, T> InnerWriteGuard<'w, T> {
    /// Takes an update which will change the state of the underlying data. This
    /// is done through the interface of UpdateTables.
    ///
    /// The return value can be anything that owns it's own data. We don't allow
    /// the return value to be a reference to the data as a way to encourage
    /// keeping the tables in sync. Since returning a &mut would allow users to
    /// cause mutations outside of the update they pass.
    ///
    /// The update passed in must be valid for 'static because it will outlive
    /// the InnerWriteGuard taking the update, so we can't make any limitations on
    /// it.
    pub fn update_tables<'a, R>(
        &'a mut self,
        mut update: impl UpdateTables<'a, T, R> + 'static + Sized + Send,
    ) -> R {
        let res = update.apply_first(&mut self.table);

        self.ops_to_replay.push(Box::new(move |table| {
            update.apply_second(table);
        }));

        res
    }

    pub fn update_tables_closure<R>(
        &mut self,
        update: impl Fn(&mut T) -> R + 'static + Sized + Send,
    ) -> R {
        let res = update(&mut self.table);
        self.ops_to_replay.push(Box::new(move |table| {
            update(table);
        }));
        res
    }
}

impl<'w, T> Drop for InnerWriteGuard<'w, T> {
    fn drop(&mut self) {
        assert!(self.blocking_readers.is_empty());

        // I initially implemented this as drop, and explicitly called
        // 'drop(table)'. This didn't actually take effect until the end
        // of this function though, causing us to record the epochs before the
        // swap had occurred. Caught by tsan.
        self.table.swap_active_and_standby();

        // Make sure that swap occurs before recording the epoch.
        fence(Ordering::SeqCst);

        for (key, epoch) in self.readers.lock().unwrap().iter_mut() {
            // Once the tables have been swapped, record the epoch of each
            // reader so that we will know if it is safe to update the new
            // standby table.
            let first_epoch_after_swap = epoch.load(Ordering::Acquire);
            if first_epoch_after_swap % 2 != 0 {
                self.blocking_readers.insert(key, first_epoch_after_swap);
            }
        }
    }
}

/// Dereferencing the InnerWriteGuard will let you see the state of the
/// standby table. If you want to inspect the state of the active_table you must
/// go through a Reader.
impl<'w, T> std::ops::Deref for InnerWriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for InnerWriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteGuard")
            .field("num_ops_to_replay", &self.ops_to_replay.len())
            .field("standby_table", &self.table)
            .finish()
    }
}

/// Writer is a wrapper around InnerWriter that can be sent and shared across
/// threads.
///
/// Sync - Writer ensures this by manually enforcing that only 1 WriteGuard can
/// be created at a time with a Mutex.
pub struct Writer<T> {
    // Used to lock 'write'. We can't use Mutex<InnerWriter> because then locking
    // would return MutexGuard<InnerWriter>. Then if we try to crete a InnerWriteGuard,
    // this would create a struct that refers to itself. Instead we use the C++
    // style unique_lock, to guard 'writer'. This is basically how Mutex is
    // implemented.
    mtx: Mutex<()>,

    // UnsafeCell is used for interior mutability.
    writer: UnsafeCell<InnerWriter<T>>,
}

impl<T> Writer<T>
where
    T: Clone,
{
    pub fn new(t: T) -> Writer<T> {
        Self::from_identical(t.clone(), t)
    }
}

impl<T> Writer<T>
where
    T: Default,
{
    pub fn default() -> Writer<T> {
        Self::from_identical(T::default(), T::default())
    }
}

impl<T> Writer<T> {
    pub fn from_identical(t1: T, t2: T) -> Writer<T> {
        Writer {
            mtx: Mutex::new(()),
            writer: UnsafeCell::new(InnerWriter::from_identical(t1, t2)),
        }
    }

    pub fn write(&self) -> WriteGuard<'_, T> {
        // Grab the mutex as the first thing.
        let _mtx_guard = Some(self.mtx.lock().unwrap());

        let writer = unsafe { &mut *self.writer.get() };
        WriteGuard {
            _mtx_guard,
            write_guard: Some(UnsafeCell::new(writer.write())),
        }
    }

    pub fn new_reader(&self) -> Reader<T> {
        let _mtx_guard = self.mtx.lock().unwrap();
        let writer = unsafe { &*self.writer.get() };
        writer.new_reader()
    }
}

impl<T: fmt::Debug> fmt::Debug for Writer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut _lock = Some(self.mtx.lock().unwrap());

        let writer = unsafe { &*self.writer.get() };
        let res = writer.fmt(f);

        // Don't delete the lock until after we read from Writer.
        std::sync::atomic::compiler_fence(Ordering::SeqCst);
        _lock = None;

        res
    }
}

/// We enforce Sync by making sure that the InnerWriteGuard only exists when the
/// MutexGuard exists.
unsafe impl<T> Sync for Writer<T> where Writer<T>: Send {}

/// Guard used for updating the tables.
pub struct WriteGuard<'w, T> {
    _mtx_guard: Option<MutexGuard<'w, ()>>,
    write_guard: Option<UnsafeCell<InnerWriteGuard<'w, T>>>,
}

impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*&*self.write_guard.as_ref().unwrap().get() }
    }
}

impl<'w, T> WriteGuard<'w, T> {
    pub fn update_tables<'a, R>(
        &'a mut self,
        update: impl UpdateTables<'a, T, R> + 'static + Sized + Send,
    ) -> R {
        unsafe { &mut *self.write_guard.as_ref().unwrap().get() }.update_tables(update)
    }

    pub fn update_tables_closure<R>(
        &mut self,
        update: impl Fn(&mut T) -> R + 'static + Sized + Send,
    ) -> R {
        unsafe { &mut *self.write_guard.as_ref().unwrap().get() }.update_tables_closure(update)
    }
}

impl<'w, T> Drop for WriteGuard<'w, T> {
    fn drop(&mut self) {
        // Make sure to destroy the write guard before the mutex.
        self.write_guard = None;
        self._mtx_guard = None;
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for WriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let wg = unsafe { &*self.write_guard.as_ref().unwrap().get() };
        wg.fmt(f)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::thread;

    struct PushVec<T> {
        value: T,
    }
    impl<'a, T> UpdateTables<'a, Vec<T>, ()> for PushVec<T>
    where
        T: Clone,
    {
        fn apply_first(&mut self, table: &'a mut Vec<T>) {
            table.push(self.value.clone());
        }
        fn apply_second(self, table: &mut Vec<T>) {
            table.push(self.value); // Move the value instead of cloning.
        }
    }

    struct PopVec {}
    impl PopVec {
        fn apply<'a, T>(&mut self, table: &'a mut Vec<T>) -> Option<T> {
            table.pop()
        }
    }
    impl<'a, T> UpdateTables<'a, Vec<T>, Option<T>> for PopVec {
        fn apply_first(&mut self, table: &'a mut Vec<T>) -> Option<T> {
            self.apply(table)
        }
        fn apply_second(mut self, table: &mut Vec<T>) {
            (&mut self).apply(table);
        }
    }

    /// This is an example of what not to do!
    struct MutableRef {}
    impl<'a, T> UpdateTables<'a, Vec<T>, &'a mut T> for MutableRef {
        fn apply_first(&mut self, table: &'a mut Vec<T>) -> &'a mut T {
            &mut table[0]
        }
        fn apply_second(self, table: &mut Vec<T>) {
            &mut table[0];
        }
    }

    #[test]
    #[should_panic(expected = "Reader is not reentrant")]
    fn reader_not_reentrant() {
        let writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        let _rg1 = reader.read();
        let _rg2 = reader.read();
    }

    #[test]
    fn one_write_guard() {
        let writer = Writer::<Vec<i32>>::default();
        let _wg = writer.write();

        // If we uncomment this line the program fails to compile due to a
        // second mutable borrow. This is what we want to guarantee there can
        // only be 1 WriteGuard at a time.
        //
        // let wg2 = writer.write();
    }

    #[test]
    fn one_read_guard() {
        let writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        let _rg = reader.read();

        // If we uncomment this line the program fails to compile due to a
        // second mutable borrow. This is an important guarantee since epoch
        // tracking is done each time a ReadGuard is created.
        //
        // let _rg2 = reader.read();
    }

    #[test]
    fn publish_update() {
        let writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        assert_eq!(reader.read().len(), 0);

        {
            let mut wg = writer.write();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(wg.len(), 1);
            assert_eq!(reader.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*reader.read(), vec![2]);
    }

    #[test]
    fn update_tables_closure() {
        let writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        assert_eq!(reader.read().len(), 0);

        {
            let mut wg = writer.write();
            wg.update_tables_closure(|vec| vec.push(2));
            assert_eq!(wg.len(), 1);
            assert_eq!(reader.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*reader.read(), vec![2]);
    }

    #[test]
    fn multi_apply() {
        let writer = Writer::<Vec<i32>>::default();
        {
            let mut wg = writer.write();
            wg.update_tables(PushVec { value: 2 });
            wg.update_tables(PushVec { value: 3 });
            wg.update_tables(PushVec { value: 4 });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: 5 });
        }
        let reader = writer.new_reader();
        assert_eq!(*reader.read(), vec![2, 3, 5]);
    }

    #[test]
    fn multi_publish() {
        let writer = Writer::<Vec<Box<i32>>>::default();
        {
            let mut wg = writer.write();
            wg.update_tables(PushVec { value: Box::new(2) });
            wg.update_tables(PushVec { value: Box::new(3) });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: Box::new(5) });
        }
        let reader = writer.new_reader();
        assert_eq!(*reader.read(), vec![Box::new(2), Box::new(5)]);

        {
            let mut wg = writer.write();
            wg.update_tables(PushVec { value: Box::new(9) });
            wg.update_tables(PushVec { value: Box::new(8) });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: Box::new(7) });
        }
        let reader = writer.new_reader();
        assert_eq!(
            *reader.read(),
            vec![Box::new(2), Box::new(5), Box::new(9), Box::new(7)]
        );

        {
            let mut wg = writer.write();
            wg.update_tables(PopVec {});
        }
        let reader = writer.new_reader();
        assert_eq!(*reader.read(), vec![Box::new(2), Box::new(5), Box::new(9)]);
    }

    #[test]
    fn multi_thread() {
        let writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        let handler = thread::spawn(move || {
            while *reader.read() != vec![2, 3, 5] {
                // Since commits oly happen when a WriteGuard is dropped no reader
                // will see this state.
                assert_ne!(*reader.read(), vec![2, 3, 4]);
            }

            // Show multiple readers in multiple threads.
            let reader2 = Reader::clone(&reader);
            let handler = thread::spawn(move || while *reader2.read() != vec![2, 3, 5] {});
            assert!(handler.join().is_ok());
        });

        {
            let mut wg = writer.write();
            wg.update_tables(PushVec { value: 2 });
            wg.update_tables(PushVec { value: 3 });
            wg.update_tables(PushVec { value: 4 });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: 5 });
        }

        assert!(handler.join().is_ok());
    }

    #[test]
    fn writer_dropped() {
        // Show that when the Writer is dropped, Readers remain valid.
        let reader;
        {
            let writer = Writer::<Vec<i32>>::default();
            reader = writer.new_reader();

            {
                let mut wg = writer.write();
                wg.update_tables(PushVec { value: 2 });
                wg.update_tables(PushVec { value: 3 });
                wg.update_tables(PushVec { value: 4 });
                wg.update_tables(PopVec {});
                wg.update_tables(PushVec { value: 5 });
            }
        }
        assert_eq!(*reader.read(), vec![2, 3, 5]);
    }

    #[test]
    fn debug_str() {
        let writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        assert_eq!(format!("{:?}", writer), "Writer { num_ops_to_replay: 0 }");
        {
            let mut wg = writer.write();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(
                format!("{:?}", wg),
                "WriteGuard { num_ops_to_replay: 1, standby_table: TableWriteGuard { standby_table: [2] } }");
        }
        assert_eq!(format!("{:?}", writer), "Writer { num_ops_to_replay: 1 }");
        assert_eq!(format!("{:?}", reader), "Reader { num_readers: 1 }");
        assert_eq!(
            format!("{:?}", reader.read()),
            "ReadGuard { active_table: [2] }"
        );
    }

    #[test]
    fn mutable_ref() {
        // Show that when the Writer is dropped, Readers remain valid.
        let writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            wg.update_tables(PushVec { value: 2 });
            let mr = wg.update_tables(MutableRef {});
            *mr = 10;
        }

        assert_eq!(*reader.read(), vec![10]);
    }
}
