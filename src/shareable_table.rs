use crate::types::*;
use std::fmt;

pub type RwLock<T> = crossbeam::sync::ShardedLock<T>;
pub type RwLockReadGuard<'r, T> = crossbeam::sync::ShardedLockReadGuard<'r, T>;
pub type RwLockWriteGuard<'w, T> = crossbeam::sync::ShardedLockWriteGuard<'w, T>;

pub struct SharedTable<T> {
    is_table0_active: RwLock<bool>,
    write_lock: Mutex<()>,
    table0: RwLock<T>,
    table1: RwLock<T>,
}

pub struct SharedTableWriteGuard<'w, T> {
    is_table0_active: &'w RwLock<bool>,
    write_guard: MutexGuard<'w, ()>,
    standby_table: RwLockWriteGuard<'w, T>,
}

impl<T> SharedTable<T> {
    pub fn new(t: T) -> SharedTable<T>
    where
        T: Clone,
    {
        SharedTable::from_identical(t.clone(), t)
    }

    pub fn from_identical(t1: T, t2: T) -> SharedTable<T> {
        SharedTable {
            is_table0_active: RwLock::new(true),
            write_lock: Mutex::default(),
            table0: RwLock::new(t1),
            table1: RwLock::new(t2),
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, T> {
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
        let mut guard = Some(self.is_table0_active.read().unwrap());
        let is_table0_active = *guard.unwrap();

        let active_table;
        if is_table0_active {
            active_table = self.table0.read().unwrap();
        } else {
            active_table = self.table1.read().unwrap();
        }

        // Make sure that the guard is only dropped after we reference the
        // table.
        std::sync::atomic::compiler_fence(Ordering::SeqCst);
        guard = None;

        active_table
    }

    pub fn write(&self) -> SharedTableWriteGuard<'_, T> {
        // Use write_guard to make sure that calls to 'write' are single
        // threaded. This keeps writers from interacting with readers other than
        // when waiting on readers to drop a pre-existing guard to the new
        // standby table.
        let write_guard = self.write_lock.lock().unwrap();
        let standby_table;
        if *self.is_table0_active.read().unwrap() {
            standby_table = self.table1.write().unwrap();
        } else {
            standby_table = self.table0.write().unwrap();
        }

        SharedTableWriteGuard {
            is_table0_active: &self.is_table0_active,
            write_guard,
            standby_table,
        }
    }
}

impl<T> SharedTableWriteGuard<'_, T> {
    /// Call when dropping the write guard.
    pub fn swap_active_and_standby(&mut self) {
        let mut guard = self.is_table0_active.write().unwrap();
        *guard = !(*guard);
    }
}

// TODO: consider adding "T: ?Sized" like std::sync::RwLock.
impl<'w, T> std::ops::Deref for SharedTableWriteGuard<'w, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.standby_table
    }
}

impl<'w, T> std::ops::DerefMut for SharedTableWriteGuard<'w, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut *self.standby_table
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for SharedTableWriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedTableWriteGuard")
            .field("standby_table", unsafe { &*self.standby_table })
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn shared_table() {
        let table = SharedTable::new(5);

        {
            let mut wg = table.write();
            *wg += 1;
            wg.swap_active_and_standby();
        }

        assert_eq!(*table.read(), 6);
    }
}
