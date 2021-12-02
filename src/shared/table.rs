use crate::types::*;
use std::fmt;

pub struct Table<T> {
    is_table0_active: RwLock<bool>,
    table0: RwLock<T>,
    table1: RwLock<T>,
}

pub struct TableWriteGuard<'w, T> {
    is_table0_active: &'w RwLock<bool>,
    standby_table: Option<RwLockWriteGuard<'w, T>>,
}

impl<T> Table<T> {
    pub fn from_identical(t1: T, t2: T) -> Table<T> {
        Table {
            is_table0_active: RwLock::new(true),
            table0: RwLock::new(t1),
            table1: RwLock::new(t2),
        }
    }

    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        // Use a read guard to make sure there is no race between:
        // - read guards reading from `is_table0_active` and grabbing a
        //   reader/writer.
        // - updating the value of `is_table0_active` when a write guard is
        //   dropped.
        //
        // If `is_table0_active` was an AtomicBool, we could reach a situation
        // where:
        // 1. A reader calls 'read' and sees that table0 is active.
        // 2. The OS pre-empts this thread before it actually grabs the read
        //    guard for table0.
        // 3. A writer grabs table1, then drops it flipping the tables.
        // 4. A writer grabs table0.
        // 5. The OS wakes up the reader thread, which is now blocked trying to
        //    get a read guard to table0.
        //
        // By using an RwLock to guard the entire call of read & write, we
        // guarantee that a reader will never get stuck waiting for a writer to
        // release a given table.
        let guard = self.is_table0_active.read().unwrap();
        let is_table0_active = *guard;

        let active_table;
        if is_table0_active {
            active_table = self.table0.read();
        } else {
            active_table = self.table1.read();
        }

        active_table
    }

    // Caller must ensure that 'write' is single threaded, or else we will block
    // readers.
    pub fn write(&self) -> TableWriteGuard<'_, T> {
        // We don't need to worry about the RwLock being fair:
        // - Only 1 WriteGuard can exist at a time, and when it updates the
        //   active and standby tables, this is done while write locking
        //   `is_table0_active`.
        // - Any reader that would attempt to gain access to what at this point
        //   is considered the standby table, must already hold a read lock to
        //   the table before we attempt to write lock it.
        // - Therefore this write lock will only contend with pre-existing read
        //   guards, never incoming attempts to read lock.
        let standby_table;
        if *self.is_table0_active.read().unwrap() {
            standby_table = self.table1.write().unwrap();
        } else {
            standby_table = self.table0.write().unwrap();
        }

        TableWriteGuard {
            is_table0_active: &self.is_table0_active,
            standby_table: Some(standby_table),
        }
    }
}

impl<T> Drop for TableWriteGuard<'_, T> {
    fn drop(&mut self) {
        // Drop the WriteGuard to standby table before write guarding
        // 'is_table0_active'. Otherwise this triggers TSAN's lock inversion
        // (deadlock detector) because in 'read' we lock 'is_table0_active' and
        // under that guard we lock the active table. Therefore here we need to
        // release the lock on the standby table before locking
        // 'is_table0_active'.
        //
        // This may be a false-positive similar to
        // https://github.com/google/sanitizers/issues/814.
        self.standby_table = None;

        let mut guard = self.is_table0_active.write().unwrap(); // !!!!!!!!!!!!!!!!!!!
        *guard = !(*guard);
    }
}

// TODO: consider adding "T: ?Sized" like std::sync::RwLock.
impl<'w, T> std::ops::Deref for TableWriteGuard<'w, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.standby_table.as_ref().unwrap()
    }
}

impl<'w, T> std::ops::DerefMut for TableWriteGuard<'w, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut *self.standby_table.as_mut().unwrap()
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for TableWriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableWriteGuard")
            .field("standby_table", &**self.standby_table.as_ref().unwrap())
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn table() {
        let table = Table::from_identical(5, 5);

        {
            let mut wg = table.write();
            *wg += 1;
        }

        assert_eq!(*table.read().unwrap(), 6);
    }
}
