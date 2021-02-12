use std::sync;
use sync::atomic::{AtomicBool, Ordering};
use sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

// TODO: Consider using crossbeam sharded RwLock since it's optimized for fast
// reads. Since reads should never be contested a faster read implementation
// seems good. The slower write lock shouldn't be an issue since the slowness on
// writes that I am worried about is due to reader threads still holding the new
// 'standby_table' when we try to create a new WriteGuard.

pub struct Table<T> {
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
    pub fn swap_active_and_standby(&mut self) {
        self.is_table0_active.store(
            !self.is_table0_active.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
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
