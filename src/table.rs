use crate::types::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::sync;
use sync::atomic::{AtomicBool, Ordering};

pub struct Table<T> {
    // Possible alternative design is to stop having the bool, and instead have
    // just the 2 tables as AtomicPtr, and WriteGuard will swap the pointers.
    // The tradeoff I am most interested in is the performance of read_guard.
    // This is a tradeoff between branching and indirection. I lean now towards
    // branching since CPUs seem to be quite fast and other examples indicate to
    // me that this could be better (aka C++ std::string switched from just
    // following a pointer to storing short strings locally
    // https://www.youtube.com/watch?v=kPR8h4-qZdk)
    table0: RwLock<T>,
    table1: RwLock<T>,

    // If true, table0 is the 'active' table, the one that readers will read
    // from on their next refresh and table1 is the 'standby' table which will
    // receive updates from a WriteGuard.
    is_table0_active: AtomicBool,
}

impl<T> Table<T> {
    // Return the peices needed by a WriteGuard.
    pub fn write_guard(&mut self) -> (RwLockWriteGuard<'_, T>, &mut AtomicBool) {
        let standby_table = if self.is_table0_active.load(Ordering::Relaxed) {
            self.table1.write()
        } else {
            self.table0.write()
        };

        (standby_table.unwrap(), &mut self.is_table0_active)
    }

    // Return the pieces needed by a ReadGuard.
    pub fn read_guard(&self) -> RwLockReadGuard<'_, T> {
        if self.is_table0_active.load(Ordering::Relaxed) {
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
    pub fn new_from_empty(t: T) -> Table<T> {
        Table {
            table0: RwLock::new(t.clone()),
            table1: RwLock::new(t),
            is_table0_active: AtomicBool::new(true),
        }
    }
}
