use crate::types::*;

/// Table holds the 2 copies of the underlying table. It is meant to abstract
/// away the details of there being 2 tables as much as is reasonable. This
/// class is deeply tied into the usage pattern Writer to guarantee correctness:
/// 1. Only 1 WriteGuard exists at a time,
/// 2. Writer handles updating both tables (ie replay),
/// 3. Writer determines when to swap active & standby.
/// 4. Writer awaits `standby_table` being free of ReadGuards before mutating.
///
/// Any number of Readers can exist, and they may only interact with table by
/// calling to `active_table()`.
///
/// All operations (mostly swap) on the active and standby tables are SeqCst,
/// since they need to be kept in sync relative to each other.
///
/// A couple other designs considered:
/// 1. AtomicPtr<(T, read_count)> - This means that the reader will grab the
///    active table before incrementing the counter, so it won't actually be
///    locked. Therefore a reader can grab the active table, then go to sleep
///    before incrementing the counter, and then have the writer do swapping and
///    only wake up while the writer holds the table.
/// 2. AtomicBool table0_active. Adding this still requires Readers to
///    synchronize with Writer outside the scope of Table. I think the reason I
///    didn't do this was to try and keep reads as fast as possible. That
///    involved another read + a branch before grabbing the table; the current
///    method is just grabbing a pointer. I think it was possible to do this
///    safely.
#[derive(Debug)]
pub struct Table<T> {
    pub active_table: AtomicPtr<T>,
    pub standby_table: AtomicPtr<T>,
}

impl<T> Table<T> {
    pub fn from_identical(t1: T, t2: T) -> Table<T> {
        Table {
            active_table: AtomicPtr::new(Box::into_raw(Box::new(t1))),
            standby_table: AtomicPtr::new(Box::into_raw(Box::new(t2))),
        }
    }

    // This is the only function that Readers should ever call to.
    pub fn active_table(&self) -> &T {
        // Memory safety (valid pointer) is guaranteed by the class. See class
        // level comment.
        //
        // Thread safety isn't guaranteed by the compiler, instead our access
        // patterns (Reader & Writer) must enforce that this table is never
        // updated by the Writer so long as this read exists.
        unsafe { &*self.active_table.load(Ordering::SeqCst) }
    }

    // Read access of the standby_table. Only to be used by the Writer.
    pub fn standby_table(&self) -> &T {
        // Memory safety (valid pointer) is guaranteed by the class. See class
        // level comment.
        //
        // This is thread safe as long as the user guarantees that:
        // 1. The standby table is only accessed by the Writer.
        // 2. Only 1 Writer will attempt to interact with the table at a time.
        unsafe { &*self.standby_table.load(Ordering::SeqCst) }
    }

    pub fn standby_table_mut(&self) -> &mut T {
        // Memory safety (valid pointer) is guaranteed by the class. See class
        // level comment.
        //
        // This is thread safe as long as the user guarantees that:
        // 1. The standby table is only accessed by the Writer.
        // 2. Only 1 Writer will attempt to interact with the table at a time.
        // 3. Writers wait for all Readers to switch off this table after calling
        //    to `swap_active_and_standby`.
        unsafe { &mut *self.standby_table.load(Ordering::SeqCst) }
    }

    /// Swap which underlying table `active_table` and `standby_table` point to.
    /// Only to be called by the Writer.
    ///
    /// Once this has been called, existing ReadGuard will be left pointing to
    /// `standby_table`. This means that the Writer must wait until all of those
    /// ReadGuards have been dropped before it is safe to mutate `standby_table`.
    pub fn swap_active_and_standby(&self) {
        let active_table = self.active_table.load(Ordering::SeqCst);
        let standby_table = self.standby_table.load(Ordering::SeqCst);
        assert_ne!(active_table, standby_table);

        // Swap the active and standby tables. These should never fail because
        // there can only ever be 1 writer which spawns only 1 WriteGuard.
        let res = self.active_table.compare_exchange(
            active_table,
            standby_table,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        assert_eq!(res, Ok(active_table));

        let res = self.standby_table.compare_exchange(
            standby_table,
            active_table,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        assert_eq!(res, Ok(standby_table));
    }
}

impl<T> Drop for Table<T> {
    fn drop(&mut self) {
        // Memory safety (valid pointer) is guaranteed by the class. See class
        // level comment.
        unsafe {
            let _active_table = Box::from_raw(self.active_table.load(Ordering::SeqCst));
            let _standby_table = Box::from_raw(self.standby_table.load(Ordering::SeqCst));
        }
    }
}
