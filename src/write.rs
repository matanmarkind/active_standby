use crate::read::Reader;
use crate::table::Table;
use crate::types::RwLockWriteGuard;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Writer is the class used to gain access to mutating the underlying tables.
/// In order to interact with the underlying tables you must create a
/// WriteGuard.
///
/// Writer doesn't actually own the underlying data, so if Writer is Dropped,
/// this will not delete the tables. Instead they will only be dropped once all
/// Readers and the Writer are dropped.
pub struct Writer<T> {
    table: Arc<Table<T>>,

    /// Log of operations to be performed on the active table. This gets played
    /// on the standby table when creating a WriteGuard as an optimization.
    /// Since when a WriteGuard is dropped, we swap the active and standby
    /// tables, by waiting until the next time a WriteGuard is created we give
    /// the readers time to switch to reading from the new active_table. This
    /// hopefully reduces contention when the writer tries to lock the new
    /// standby_table.
    ///
    /// We could make the Writer Send + Sync if we instead gave up on this
    /// optimization and moved ops_to_replay into WriteGuard, and had WriteGuard
    /// perform these ops on Drop. I think this optimization is worth the need
    /// for the user to wrap Writer in a Mutex though.
    ops_to_replay: Vec<Box<dyn FnOnce(&mut T)>>,
}

impl<T> Writer<T>
where
    T: Clone,
{
    pub fn new_from_empty(t: T) -> Writer<T> {
        Writer {
            table: Arc::new(Table::new_from_empty(t)),
            ops_to_replay: vec![],
        }
    }
}

impl<T> Writer<T>
where
    T: Default + Clone,
{
    pub fn default() -> Writer<T> {
        Self::new_from_empty(T::default())
    }
}

impl<T> Writer<T> {
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
        // We rely on knowing that this is the only Writer and it can only call
        // to 'write' when there are no existing WriteGuards.
        let table = unsafe {
            std::mem::transmute::<*const Table<T>, &mut Table<T>>(Arc::as_ptr(&self.table))
        };

        // Replay all ops on the standby table. This will hang until all readers
        // have returned their read guard.
        let (mut standby_table, is_table0_active) = table.write_guard();
        for op in self.ops_to_replay.drain(..) {
            op(&mut standby_table);
        }
        self.ops_to_replay.clear();

        WriteGuard {
            standby_table,
            ops_to_replay: &mut self.ops_to_replay,
            is_table0_active,
        }
    }

    pub fn new_reader(&self) -> Reader<T> {
        Reader::new(Arc::clone(&self.table))
    }
}

impl<T: fmt::Debug> fmt::Debug for Writer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("num_ops_to_replay", &self.ops_to_replay.len())
            .field("active_table_reader", &self.new_reader())
            .finish()
    }
}

/// WriteGuard is the way to mutate the underlying tables. A Writer can only
/// generate 1 at a time, which is enforced by the borrow checker on creation.
///
/// Unlike an RwLockWriteGuard, we don't mutate the underlying data in a
/// transparent manner. Instead the caller must pass in a function which
/// implements the UpdateTables trait to mutate the underlying data.
///
/// When dereferencing a WriteGuard we see the state of the standby_table, not
/// the active_table which the Readers dereference.
///
/// Upon Drop, a WriteGuard automatically publishes the changes to the Readers,
/// by swapping the active and standby tables. The updates are only performed on
/// the new standby table the next time a WriteGuard is created. This is to
/// minimize thread contention. That way Readers will have a chance to switch to
/// reading from the new active table before trying to WriteLock the new standby
/// table.
pub struct WriteGuard<'w, T> {
    standby_table: RwLockWriteGuard<'w, T>,

    // Record the ops that were applied to standby_table to be replayed the next
    // time we create a WriteGuard.
    ops_to_replay: &'w mut Vec<Box<dyn FnOnce(&mut T)>>,

    // Updated at drop.
    is_table0_active: &'w mut AtomicBool,
}

/// WriteGuard is likely to be the trickiest for use. It is critical that the
/// user make sure that any mutation that occurs on one table, also occurs on
/// the other. In order to achieve this we provider 3 interfaces:
/// - update_tables - the simplest and safest interface. Takes in a single
///   function and applies it to both tables.
/// - update_tables_individually - allows for more complex return values
///   specifically with lifetime requirements.
/// - standby_table_and_active_table_enqueue - the least preffered interface due
///   to expectations of misuse. The user must make sure that all mutations
///   performed on the standby_table directly are reflected in the update
///   operation(s) they enqueue.
impl<'w, T> WriteGuard<'w, T> {
    /// Passes in a function to mutate the tables that is performed on both
    /// tables. The operation is applied synchronously on the standby_table and
    /// the return value is returned to the caller. The op is then enqueued and
    /// will be called on the current active_table before the next WriteGuard is
    /// created (when it will be the standby_table).
    ///
    /// Please be aware that any mutations that the caller makes on a returned
    /// value that affect the underlying table will not be reflected when the
    /// tables swap since we only replay the function, we don't know what the
    /// caller will do with it. That is why the return value is not tied to the
    /// lifetime of self, in the hopes this will prevent that from being
    /// possible.
    pub fn update_tables<R, F>(&mut self, mut update: F) -> R
    where
        F: 'static + FnMut(&mut T) -> R,
    {
        let res = update(&mut self.standby_table);
        self.ops_to_replay.push(Box::new(move |table| {
            update(table);
        }));
        res
    }
}

/// When the WriteGuard is dropped we swap the active and standby tables. We
/// don't update the new standby table until a new WriteGuard is created.
impl<'w, T> Drop for WriteGuard<'w, T> {
    fn drop(&mut self) {
        // Make sure to drop the write guard first to guarantee that readers
        // never face contention.
        drop(&mut self.standby_table);

        // Swap the active and standby tables.
        self.is_table0_active.store(
            !self.is_table0_active.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
}

/// Dereferencing the WriteGuard will let you see the state of the
/// standby_table. If you want to inspect the state of the active_table you must
/// go through a Reader.
impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &*self.standby_table
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for WriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteGuard")
            .field("num_ops_to_replay", &self.ops_to_replay.len())
            .field("is_table0_active", &self.is_table0_active)
            .field("standby_table", &self.standby_table)
            .finish()
    }
}

/// Writer which is Send + Sync by just wrapping a Writer in a Mutex.
///
/// Given that I have to explicitly mark this as Send + Sync I am a little
/// worried about this struct.
pub struct SyncWriter<T> {
    writer: Mutex<Writer<T>>,
}

unsafe impl<T> Send for SyncWriter<T> {}
unsafe impl<T> Sync for SyncWriter<T> {}

impl<T> std::ops::Deref for SyncWriter<T> {
    type Target = Mutex<Writer<T>>;
    fn deref(&self) -> &Self::Target {
        &self.writer
    }
}

impl<T> std::ops::DerefMut for SyncWriter<T> {
    fn deref_mut(&mut self) -> &mut Mutex<Writer<T>> {
        &mut self.writer
    }
}

impl<T: fmt::Debug> fmt::Debug for SyncWriter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncWriter")
            .field("writer", &self.writer)
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::thread;

    fn check_tables_match<T: PartialEq + std::fmt::Debug>(writer: &mut Writer<T>, expected: T) {
        assert_eq!(*writer.new_reader().read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*writer.new_reader().read(), expected);
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
            wg.update_tables(|table: &mut Vec<i32>| table.push(2));
            assert_eq!(
                wg.update_tables(|table: &mut Vec<i32>| table.pop()),
                Some(2)
            );
            wg.update_tables(|table: &mut Vec<i32>| table.push(4));

            assert_eq!(wg.len(), 1);
            assert_eq!(reader.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        check_tables_match(&mut writer, vec![4]);
    }

    #[test]
    fn multi_apply() {
        // As opposed to the above which could mask an issue of just applying
        // the last update, show multiple updates with their side effects.
        let mut writer = Writer::<Vec<i32>>::default();
        {
            let mut wg = writer.write();
            wg.update_tables(|table: &mut Vec<i32>| table.push(2));
            wg.update_tables(|table: &mut Vec<i32>| table.push(3));
            wg.update_tables(|table: &mut Vec<i32>| table.push(4));
            wg.update_tables(|table: &mut Vec<i32>| table.pop());
            wg.update_tables(|table: &mut Vec<i32>| table.push(5));
            assert_eq!(*wg, vec![2, 3, 5]);
        }

        check_tables_match(&mut writer, vec![2, 3, 5]);
    }

    #[test]
    fn multi_publish() {
        let mut writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.update_tables(|table: &mut Vec<i32>| table.push(4));
        }
        assert_eq!(*reader.read(), vec![4]);

        {
            let mut wg = writer.write();
            wg.update_tables(|table: &mut Vec<i32>| table.push(4));
        }
        assert_eq!(*reader.read(), vec![4, 4]);

        {
            let mut wg = writer.write();
            wg.update_tables(|table: &mut Vec<i32>| table.pop());
        }
        assert_eq!(*reader.read(), vec![4]);
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
            wg.update_tables(|table: &mut Vec<i32>| table.push(2));
            wg.update_tables(|table: &mut Vec<i32>| table.push(3));
            wg.update_tables(|table: &mut Vec<i32>| table.push(4));
            wg.update_tables(|table: &mut Vec<i32>| table.pop());
            wg.update_tables(|table: &mut Vec<i32>| table.push(5));
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
                wg.update_tables(|table: &mut Vec<i32>| table.push(2));
                wg.update_tables(|table: &mut Vec<i32>| table.push(3));
                wg.update_tables(|table: &mut Vec<i32>| table.push(4));
                wg.update_tables(|table: &mut Vec<i32>| table.pop());
                wg.update_tables(|table: &mut Vec<i32>| table.push(5));
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
            wg.update_tables(|table: &mut Vec<i32>| table.push(2));
            assert_eq!(
                format!("{:?}", wg),
                "WriteGuard { num_ops_to_replay: 1, is_table0_active: true, standby_table: RwLockWriteGuard { lock: RwLock { data: <locked> } } }");
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
