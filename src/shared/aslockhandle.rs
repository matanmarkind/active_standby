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
    // synchronization across Writer/Readers, rather that is handled by the
    // Writer and Readers themselves.
    table: Arc<Table<T>>,

    /// Log of operations to be performed on the second table. This gets played
    /// on the standby table when creating a WriteGuard, as opposed to when
    /// dropping it, to minimize lock contention. This is in the hopes that by
    /// waiting until the next time a WriteGuard is created, we give the readers
    /// time to switch to reading from the new active_table.
    ops_to_replay: Mutex<Vec<Box<dyn FnOnce(&mut T) + Send>>>,
}

/// Guard for a SyncWriter, not a WriteGuard that is Sync.
///
/// Same as a WriteGuard, but update_tables requires that updates are Send.
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
            table: Arc::new(Table::from_identical(t1, t2)),
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
    pub fn write(&mut self) -> WriteGuard<'_, T> {
        let mut table = self.table.write();

        // Replay all ops on the standby table.
        let mut ops_to_replay = self.ops_to_replay.lock().unwrap();
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

impl<T: fmt::Debug> fmt::Debug for AsLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsLock").finish()
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
