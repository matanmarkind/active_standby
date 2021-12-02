// use crate::shared::read::{Reader, ReaderEpochs};
use crate::shared::table::{Table, TableWriteGuard};
use crate::types::*;
use std::fmt;

/// Struct for holding tables that can be interacted with like an RwLock,
/// including being shared across threads/tasks via Arc (as opposed to the
/// lockless version which requires independent copies per task).
pub struct AsLock<T> {
    // The underlying tables. This struct is responsible for returning the
    // correct active/standby table. The call. This table does not handle any
    // synchronization across AsLock/Readers, rather that is handled by the
    // AsLock and Readers themselves.
    table: Table<T>,

    /// Log of operations to be performed on the second table. This gets played
    /// on the standby table when creating a WriteGuard, as opposed to when
    /// dropping it, to minimize lock contention. This is in the hopes that by
    /// waiting until the next time a WriteGuard is created, we give the readers
    /// time to switch to reading from the new active_table.
    ///
    /// This mutex is used to guarantee that 'write' is single threaded, and so
    /// locking it must be done before calling to 'table.write'.
    ops_to_replay: Mutex<Vec<Box<dyn FnOnce(&mut T) + Send>>>,
}

/// Guard used for updating the tables.
pub struct WriteGuard<'w, T> {
    table: TableWriteGuard<'w, T>,
    ops_to_replay: MutexGuard<'w, Vec<Box<dyn FnOnce(&mut T) + Send>>>,
}

impl<T> AsLock<T>
where
    T: Clone,
{
    pub fn new(t: T) -> AsLock<T> {
        Self::from_identical(t.clone(), t)
    }
}

impl<T> AsLock<T>
where
    T: Default,
{
    pub fn default() -> AsLock<T> {
        Self::from_identical(T::default(), T::default())
    }
}

impl<T> AsLock<T> {
    /// Create a AsLock object for handling active_standby tables.
    /// - t1 & t2 are the two tables which will become the active and standby
    ///   tables. They must be identical; this is left to the user to enforce.
    pub fn from_identical(t1: T, t2: T) -> AsLock<T> {
        AsLock {
            table: Table::from_identical(t1, t2),
            ops_to_replay: Mutex::default(),
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
    pub fn _write(&self) -> WriteGuard<'_, T> {
        // Grab ops_to_replay as the first thing in 'write' as a way to ensure
        // that it is single threaded.
        let mut ops_to_replay = self.ops_to_replay.lock().unwrap();

        let mut table = self.table.write();

        // Replay all ops on the standby table.
        for op in ops_to_replay.drain(..) {
            op(&mut table);
        }
        ops_to_replay.clear();

        WriteGuard {
            ops_to_replay,
            table,
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        self.table.read()
    }
}

#[cfg(active_standby_compare_tables_equal)]
impl<T> AsLock<T>
where
    T: PartialEq + std::fmt::Debug,
{
    pub fn write(&self) -> WriteGuard<'_, T> {
        let wg = self._write();
        assert_eq!(*wg, *self.read());
        wg
    }
}

#[cfg(not(active_standby_compare_tables_equal))]
impl<T> AsLock<T> {
    pub fn write(&self) -> WriteGuard<'_, T> {
        self._write()
    }
}

impl<T: fmt::Debug> fmt::Debug for AsLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let num_ops_to_replay = self.ops_to_replay.lock().unwrap().len();
        f.debug_struct("AsLock")
            .field("num_ops_to_replay", &num_ops_to_replay)
            .finish()
    }
}

impl<'w, T> WriteGuard<'w, T> {
    /// Takes an update which will change the state of the underlying data. This
    /// is done through the interface of UpdateTables.
    ///
    /// The return value can be anything that owns it's own data. We don't allow
    /// the return value to be a reference to the data as a way to encourage
    /// keeping the tables in sync. Since returning a &mut would allow users to
    /// cause mutations outside of the update they pass.
    ///
    /// The update passed in must be valid for 'static because it will outlive
    /// the WriteGuard taking the update, so we can't make any limitations on
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

/// Dereferencing the WriteGuard will let you see the state of the
/// standby table. If you want to inspect the state of the active_table you must
/// go through a Reader.
impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for WriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteGuard")
            .field("num_ops_to_replay", &self.ops_to_replay.len())
            .field("standby_table", &self.table)
            .finish()
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
    fn one_write_guard() {
        // TODO: Have a multithreaded test for this.
        let writer = AsLock::<Vec<i32>>::default();
        let _wg = writer.write();
        // let wg2 = writer.write();
    }

    #[test]
    fn publish_update() {
        let aslock = Arc::new(AsLock::<Vec<i32>>::default());
        assert_eq!(aslock.read().len(), 0);

        {
            let mut wg = aslock.write();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(wg.len(), 1);
            {
                // Perform check in another thread to avoid potential deadlock
                // (calling both read and write on aslock at the same time).
                let aslock = Arc::clone(&aslock);
                thread::spawn(move || {
                    assert_eq!(aslock.read().len(), 0);
                })
                .join()
                .unwrap();
            }
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*aslock.read(), vec![2]);
    }

    #[test]
    fn update_tables_closure() {
        let aslock = Arc::new(AsLock::<Vec<i32>>::default());
        assert_eq!(aslock.read().len(), 0);

        {
            let mut wg = aslock.write();
            wg.update_tables_closure(|vec| vec.push(2));
            assert_eq!(wg.len(), 1);
            {
                // Perform check in another thread to avoid potential deadlock
                // (calling both read and write on aslock at the same time).
                let aslock = Arc::clone(&aslock);
                thread::spawn(move || {
                    assert_eq!(aslock.read().len(), 0);
                })
                .join()
                .unwrap();
            }
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*aslock.read(), vec![2]);
    }

    #[test]
    fn multi_apply() {
        let aslock = AsLock::<Vec<i32>>::default();
        {
            let mut wg = aslock.write();
            wg.update_tables(PushVec { value: 2 });
            wg.update_tables(PushVec { value: 3 });
            wg.update_tables(PushVec { value: 4 });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: 5 });
        }
        assert_eq!(*aslock.read(), vec![2, 3, 5]);
    }

    #[test]
    fn multi_publish() {
        let aslock = AsLock::<Vec<Box<i32>>>::default();
        {
            let mut wg = aslock.write();
            wg.update_tables(PushVec { value: Box::new(2) });
            wg.update_tables(PushVec { value: Box::new(3) });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: Box::new(5) });
        }
        assert_eq!(*aslock.read(), vec![Box::new(2), Box::new(5)]);

        {
            let mut wg = aslock.write();
            wg.update_tables(PushVec { value: Box::new(9) });
            wg.update_tables(PushVec { value: Box::new(8) });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: Box::new(7) });
        }
        assert_eq!(
            *aslock.read(),
            vec![Box::new(2), Box::new(5), Box::new(9), Box::new(7)]
        );

        {
            let mut wg = aslock.write();
            wg.update_tables(PopVec {});
        }
        assert_eq!(*aslock.read(), vec![Box::new(2), Box::new(5), Box::new(9)]);
    }

    #[test]
    fn multi_thread() {
        let aslock = Arc::new(AsLock::<Vec<i32>>::default());
        let aslock2 = Arc::clone(&aslock);
        let handler = thread::spawn(move || {
            while *aslock2.read() != vec![2, 3, 5] {
                // Since commits oly happen when a WriteGuard is dropped no reader
                // will see this state.
                assert_ne!(*aslock2.read(), vec![2, 3, 4]);
            }

            // Show multiple readers in multiple threads.
            let aslock3 = Arc::clone(&aslock2);
            let handler = thread::spawn(move || while *aslock3.read() != vec![2, 3, 5] {});
            assert!(handler.join().is_ok());
        });

        {
            let mut wg = aslock.write();
            wg.update_tables(PushVec { value: 2 });
            wg.update_tables(PushVec { value: 3 });
            wg.update_tables(PushVec { value: 4 });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: 5 });
        }

        assert!(handler.join().is_ok());
    }

    #[test]
    fn debug_str() {
        let aslock = AsLock::<Vec<i32>>::default();
        assert_eq!(format!("{:?}", aslock), "AsLock { num_ops_to_replay: 0 }");
        {
            let mut wg = aslock.write();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(
                format!("{:?}", wg),
                "WriteGuard { num_ops_to_replay: 1, standby_table: TableWriteGuard { standby_table: [2] } }");
        }
        assert_eq!(format!("{:?}", aslock), "AsLock { num_ops_to_replay: 1 }");
        // The aliased shared lock shows up in this debug. What we mostly care
        // about is that this says ReadGuard and shows the underlying data. It's
        // fine to update this if we ever change the underlying RwLock.
        assert_eq!(
            format!("{:?}", aslock.read()),
            "ShardedLockReadGuard { lock: ShardedLock { data: [2] } }"
        );
    }
}
