use crate::read::{Reader, ReaderEpochInfos};
use crate::table::{Table, TableWriteGuard};
use more_asserts::*;
use slab::Slab;
use std::fmt;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// Operations that update that data held internally must implement this
/// interface.
///
/// Users must be careful to guarantee that apply_first and apply_second cause
/// the tables to end up in the same state.
pub trait UpdateTables<T, R> {
    fn apply_first(&mut self, table: &mut T) -> R;

    fn apply_second(mut self: Box<Self>, table: &mut T) {
        Self::apply_first(&mut self, table);
    }
}

/// Writer is the class used to gain access to mutating the underlying tables.
/// In order to interact with the underlying tables you must create a
/// WriteGuard.
///
/// Writer doesn't actually own the underlying data, so if Writer is Dropped,
/// this will not delete the tables. Instead they will only be dropped once all
/// Readers and the Writer are dropped.
///
/// For examples of using Writer check out the tests in this file.
pub struct Writer<T> {
    // The underlying tables. This struct is responsible for returning the
    // correct active/standby table, and also for swapping them when the
    // TableWriteGuard is dropped. This table is not responsible for sync'ing
    // across writers and readers, that is left to the Writer, to guarantee that
    // there are no ReadGuards pointing to the standby_table.
    table: Arc<Table<T>>,

    /// Information about each of the readers. Used by the Writer and Readers to
    /// synchronize so that the Writer never mutates a table that a ReadGuard is
    /// pointing to.
    readers: ReaderEpochInfos,

    /// Log of operations to be performed on the active table. This gets played
    /// on the standby table when creating a WriteGuard, as opposed to when
    /// dropping it, to minimize lock contention. This is in the hopes that by
    /// waiting until the next time a WriteGuard is created, we give the readers
    /// time to switch to reading from the new active_table.
    ///
    /// We could make the Writer Send + Sync if we instead gave up on this
    /// optimization and moved ops_to_replay into WriteGuard, and had WriteGuard
    /// perform these ops on Drop. I think this optimization is worth the need
    /// for the user to use SendWriter and wrap it in a Mutex though. The Mutex
    /// cost is minimal anyways since you will contend on locking Writer instead
    /// of Writer.write, so there is no added contention.
    ops_to_replay: Vec<Box<dyn FnOnce(&mut T)>>,
}

impl<T> Writer<T>
where
    T: Clone,
{
    pub fn new(t: T) -> Writer<T> {
        Writer {
            table: Arc::new(Table::new(t)),
            readers: Arc::new(Mutex::new(Slab::with_capacity(1024))),
            ops_to_replay: Vec::new(),
        }
    }
}

impl<T> Writer<T>
where
    T: Default + Clone,
{
    pub fn default() -> Writer<T> {
        Self::new(T::default())
    }
}

impl<T> Writer<T> {
    // Hangs until the standby table has no readers pointing to it, meaning it
    // is safe for updating.
    pub fn await_standby_table_free(&mut self) {
        // Iterate through the readers to check if there are any that will block
        // the writer.
        let mut blocking_readers = std::collections::HashSet::<usize>::new();

        // We start here instead of simply building blocking_readers out of all
        // keys as an optimistic optimization. This way if there is no-one
        // blocking the writer we don't have to spend time building the HashSet
        // (specifically hoping that this avoids any memory allocations.).
        for (key, info) in self.readers.lock().unwrap().iter() {
            let epoch = info.epoch.load(Ordering::Acquire);
            let first_epoch_after_update = info.first_epoch_after_update.load(Ordering::Acquire);

            // If the epoch has increased since this readers table was
            // swapped, then this means the reader has moved on to using the
            // new table. If the epoch was odd after the update, then this
            // means that the reader wasn't using the standby table at some
            // point since the swap, meaning that the current or next usage
            // must be from the new active table.
            if epoch <= first_epoch_after_update && first_epoch_after_update % 2 != 0 {
                blocking_readers.insert(key);
            }
        }

        // Wait until no reader is making use of the standby table.
        while !blocking_readers.is_empty() {
            blocking_readers.retain(|key| {
                let info = self.readers.lock().unwrap()[*key].clone();
                let epoch = info.epoch.load(Ordering::Acquire);
                let first_epoch_after_update =
                    info.first_epoch_after_update.load(Ordering::Acquire);

                epoch <= first_epoch_after_update && first_epoch_after_update % 2 != 0
            });

            // Make sure that the above check and the readers lock going out of
            // scope happens before the next check and yield.
            std::sync::atomic::fence(Ordering::SeqCst);

            if !blocking_readers.is_empty() {
                // Instead of busy looping we will yield this thread and come
                // back when the OS returns to us.
                std::thread::yield_now();
            }
        }
    }

    /// Create a WriteGuard to allow users to update the the data. There will
    /// only be 1 WriteGuard at a time.
    ///
    /// This function may be slow because:
    /// 1. Lock contention on the standby_table. This can occur if a ReadGuard
    ///    which was created before the last WriteGuard was dropped, still has
    ///    not itself been dropped.
    /// 2. Replaying all of the updates that were applied to the last
    ///    WriteGuard.
    pub fn write(&mut self) -> WriteGuard<'_, T> {
        // We rely on knowing that this is the only Writer (not Copy or Clone)
        // and it can only call to 'write' when there are no existing
        // WriteGuards (enforced by lifetimes). Since the next line is to grab
        // a write guard and 'is_table0_active' is atomic, there isn't in
        // practice thread unsafety if there were multiple Writers, but it
        // could create contention:
        // 1. write_guard1 begins drop. It first drops the actual write guard
        //    to the table.
        // 2. write_guard2 is created with a lock on the same table as
        //    write_guard1 was using.
        // 3. write_guard1 swaps the standby and active tables, and completes
        //    its drop.
        // 4. Readers now will try to get a read guard to the same table that
        //    write_guard2 is holding.

        // Wait until the standby table is free for us to update.
        self.await_standby_table_free();

        let mut standby_table = self.table.write();
        // Replay all ops on the standby table.
        for op in self.ops_to_replay.drain(..) {
            op(&mut standby_table);
        }
        self.ops_to_replay.clear();

        WriteGuard {
            standby_table,
            ops_to_replay: &mut self.ops_to_replay,
            readers: &self.readers,
        }
    }

    pub fn new_reader(&self) -> Reader<T> {
        Reader::new(Arc::clone(&self.readers), Arc::clone(&self.table))
    }
}

impl<T: fmt::Debug> fmt::Debug for Writer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("num_ops_to_replay", &self.ops_to_replay.len())
            .finish()
    }
}

/// WriteGuard is the interface used to actually mutate the tables. The
/// lifecycle of a WriteGuard is as follows:
/// 1. Creation - Write lock the standby_table and apply all updates on it via
///    'apply_second'. This consumes all of the udpates.
/// 2. Duration - update the standby table synchronously as updates come in via
///    'apply_first'. Updates are then held onto for the next WriteGuard to
///    apply on the other table.
/// 3. Drop - When a WriteGuard is dropped swap the active and standby tables,
///    publishing all of the updates to the Readers.
///
/// Only 1 WriteGuard can exist at a time.
pub struct WriteGuard<'w, T> {
    // The table that will be updated. On Drop, this table will be passed into
    // each of the readers as the new active table.
    standby_table: TableWriteGuard<'w, T>,

    // Record the ops that were applied to standby_table to be replayed the next
    // time we create a WriteGuard.
    ops_to_replay: &'w mut Vec<Box<dyn FnOnce(&mut T)>>,

    // Update first_epoch_after_update.
    readers: &'w ReaderEpochInfos,
}

impl<'w, T> WriteGuard<'w, T> {
    /// Takes an update which will change the state of the underlying data. This
    /// is done through the interface of UpdateTables.
    ///
    /// The return value can be anything that owns it's own data. We don't all
    /// the return value to be a reference to the data as a way to encourage
    /// keeping the tables in sync. Since returning a &mut would allow users to
    /// cause mutations outside of the update they pass.
    ///
    /// The update passed in must be valid for 'static because it will outlive
    /// the WriteGuard taking the update, so we can't make any limitations on
    /// it.
    pub fn update_tables<R>(&mut self, mut update: impl UpdateTables<T, R> + 'static + Sized) -> R {
        let res = update.apply_first(&mut self.standby_table);

        self.ops_to_replay.push(Box::new(move |table| {
            Box::new(update).apply_second(table);
        }));

        res
    }
}

impl<'w, T> Drop for WriteGuard<'w, T> {
    fn drop(&mut self) {
        // Swap the active and standby table.
        drop(&mut self.standby_table);

        for (_, info) in self.readers.lock().unwrap().iter_mut() {
            // Make sure that swap occurs before recording the epoch.
            std::sync::atomic::fence(Ordering::SeqCst);

            // Once the tables have been swapped, record the epoch of each
            // reader so that we will know if it is safe to update the new
            // standby table.
            debug_assert_le!(
                info.first_epoch_after_update.load(Ordering::Acquire),
                info.epoch.load(Ordering::Acquire)
            );
            info.first_epoch_after_update
                .store(info.epoch.load(Ordering::Acquire), Ordering::Release);
        }
    }
}

/// Dereferencing the WriteGuard will let you see the state of the
/// standby_table. If you want to inspect the state of the active_table you must
/// go through a Reader.
impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.standby_table
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for WriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteGuard")
            .field("num_ops_to_replay", &self.ops_to_replay.len())
            .finish()
    }
}

/// SendWriter is a wrapper around Writer that is able to be sent across
/// threads.
///
/// The only difference between SendWriter and Writer is that update_tables
/// requires that the update passed in be Send.
///
/// For examples with SendWriter check out the tests in the collections module.
#[derive(Debug)]
pub struct SendWriter<T> {
    writer: Writer<T>,
}

impl<T> SendWriter<T>
where
    T: Clone,
{
    pub fn new(t: T) -> SendWriter<T> {
        SendWriter {
            writer: Writer::new(t),
        }
    }
}

impl<T> SendWriter<T>
where
    T: Default + Clone,
{
    pub fn default() -> SendWriter<T> {
        Self::new(T::default())
    }
}

impl<T> SendWriter<T> {
    pub fn write(&mut self) -> SendWriteGuard<'_, T> {
        SendWriteGuard {
            guard: self.writer.write(),
        }
    }
}

impl<T> std::ops::Deref for SendWriter<T> {
    type Target = Writer<T>;
    fn deref(&self) -> &Self::Target {
        &self.writer
    }
}

/// Writer is made of 2 components.
/// - Arc<Tables> which is Send + Sync if T is Send.
/// - Vec<Updates> which is Send if the updates are.
///
/// We enforce that all updates passed to a SendWriteGuard are Send, so
/// therefore SendWriter is Send if T is.
unsafe impl<T> Send for SendWriter<T> where T: Send {}

/// Guard for a SendWriter, not a WriteGuard that is Send.
///
/// Same as a WriteGuard, but update_tables requires that updates are Send.
pub struct SendWriteGuard<'w, T> {
    guard: WriteGuard<'w, T>,
}

impl<'w, T> SendWriteGuard<'w, T> {
    pub fn update_tables<R>(
        &mut self,
        update: impl UpdateTables<T, R> + 'static + Sized + Send,
    ) -> R {
        self.guard.update_tables(update)
    }
}

impl<'w, T> std::ops::Deref for SendWriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &*self.guard
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::thread;

    struct PushVec<T> {
        value: T,
    }
    impl<T> UpdateTables<Vec<T>, ()> for PushVec<T>
    where
        T: Clone,
    {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            table.push(self.value.clone());
        }
        fn apply_second(self: Box<Self>, table: &mut Vec<T>) {
            table.push(self.value); // Move the value instead of cloning.
        }
    }

    struct PopVec {}
    impl<T> UpdateTables<Vec<T>, Option<T>> for PopVec {
        fn apply_first(&mut self, table: &mut Vec<T>) -> Option<T> {
            table.pop()
        }
    }

    #[test]
    fn one_guard() {
        let mut writer = Writer::<Vec<i32>>::default();
        let _wg = writer.write();

        // If we uncomment this line the program fails to compile due to a
        // second mutable borrow. This is what we want to guarantee there can
        // only be 1 WriteGuard at a time.
        //
        // let wg2 = writer.write();
    }

    #[test]
    fn publish_update() {
        let mut writer = Writer::<Vec<i32>>::default();
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
    fn multi_apply() {
        let mut writer = Writer::<Vec<i32>>::default();
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
        let mut writer = Writer::<Vec<Box<i32>>>::default();
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
        let mut writer = Writer::<Vec<i32>>::default();
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
            let mut writer = Writer::<Vec<i32>>::default();
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
        let mut writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        assert_eq!(
            format!("{:?}", writer),
            "Writer { num_ops_to_replay: 0, active_table_reader: Reader { read_guard: RwLockReadGuard { lock: RwLock { data: [] } } } }");
        {
            let mut wg = writer.write();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(
                format!("{:?}", wg),
                "WriteGuard { num_ops_to_replay: 1, standby_table: RwLockWriteGuard { is_table0_active: true, standby_table: RwLockWriteGuard { lock: RwLock { data: <locked> } } } }");
        }
        assert_eq!(
            format!("{:?}", writer),
            "Writer { num_ops_to_replay: 1, active_table_reader: Reader { read_guard: RwLockReadGuard { lock: RwLock { data: [2] } } } }");
        assert_eq!(
            format!("{:?}", reader),
            "Reader { read_guard: RwLockReadGuard { lock: RwLock { data: [2] } } }"
        );
    }
}
