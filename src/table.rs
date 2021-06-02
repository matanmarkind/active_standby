use crossbeam;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

// Define locally the lock types used incase we want to switch to a different
// implementation.
pub type RwLock<T> = crossbeam::sync::ShardedLock<T>;
pub type RwLockReadGuard<'r, T> = crossbeam::sync::ShardedLockReadGuard<'r, T>;

// Struct which handled write locking the table. Meant to look identical to the
// standard RwLockWriteGuard, except that internally it makes sure to swap the
// active and standby tables when dropped.
//
// TODO: consider adding T: ?Sized + 'a like std::sync::RwLockWriteGuard.
pub struct RwLockWriteGuard<'a, T> {
    standby_table: crossbeam::sync::ShardedLockWriteGuard<'a, T>,
    is_table0_active: &'a AtomicBool,
    is_table0_active_cached: bool,
}

/// When the RwLockWriteGuard is dropped we swap the active and standby tables. We
/// don't update the new standby table until a new RwLockWriteGuard is created.
impl<T> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        // Make sure to drop the write guard first to guarantee that readers
        // never face contention.
        drop(&mut self.standby_table);

        // Make sure that drop occurs before swapping active and standby.
        // TODO: Look into relaxing the ordering.
        std::sync::atomic::fence(Ordering::SeqCst);

        // Swap the active and standby tables.
        // TODO: Look into relaxing the ordering.
        let res = self.is_table0_active.compare_exchange(
            self.is_table0_active_cached,
            !self.is_table0_active_cached,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        assert_eq!(res, Ok(self.is_table0_active_cached));
    }
}

impl<'w, T> std::ops::Deref for RwLockWriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &*self.standby_table
    }
}

impl<'w, T> std::ops::DerefMut for RwLockWriteGuard<'w, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut *self.standby_table
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for RwLockWriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RwLockWriteGuard")
            .field("is_table0_active", &self.is_table0_active)
            .field("standby_table", &self.standby_table)
            .finish()
    }
}

pub struct Table<T> {
    // Possible alternative design is to stop having the bool, and instead have
    // just the 2 tables as AtomicPtr, and WriteGuard will swap the pointers.
    // The tradeoff I am most interested in is the performance of read_guard.
    // This is a tradeoff between branching and indirection (aka CPU speed v.
    // Memory loading speed). I lean now towards branching since CPUs seem to be
    // quite fast and other examples indicate to me that this could be better
    // than having AtomicPtr<RwLock<T>>. e.g. C++ std::string switched from
    // just following a pointer to storing short strings locally
    // https://www.youtube.com/watch?v=kPR8h4-qZdk
    table0: RwLock<T>,
    table1: RwLock<T>,

    // If true, table0 is the 'active' table, the one that readers will read
    // from on their next refresh and table1 is the 'standby' table which will
    // receive updates from a WriteGuard.
    is_table0_active: AtomicBool,
}

/// This marks that Table is Sync, since it handles its own synchronization. It
/// is also important to mark it this way so that Writer can be Send without T
/// needing to be Sync.
unsafe impl<T> Sync for Table<T> where T: Send {}

impl<T> Table<T> {
    // Return the peices needed by a WriteGuard.
    // TODO: Write my own WriteGuard which handles the bool on drop.
    pub fn write(&mut self) -> RwLockWriteGuard<'_, T> {
        let is_table0_active_cached = self.is_table0_active.load(Ordering::SeqCst);
        let standby_table = if is_table0_active_cached {
            self.table1.write()
        } else {
            self.table0.write()
        };

        RwLockWriteGuard {
            standby_table: standby_table.unwrap(),
            is_table0_active: &mut self.is_table0_active,
            is_table0_active_cached,
        }
    }

    // Return the pieces needed by a ReadGuard. A read guard to the
    // active_table.
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        if self.is_table0_active.load(Ordering::SeqCst) {
            self.table0.read().unwrap()
        } else {
            self.table1.read().unwrap()
        }
    }
}

impl<T> Table<T>
where
    T: Clone,
{
    pub fn new(t: T) -> Table<T> {
        Table {
            table0: RwLock::new(t.clone()),
            table1: RwLock::new(t),
            is_table0_active: AtomicBool::new(true),
        }
    }
}
