use crate::types::*;
use std::fmt;

/// Table holds the 2 copies of the underlying table, which the user should
/// experience as a single table.
///
/// Table guarantees the memory safety of the table pointers, by instantiating
/// them on construction and destroying them on drop. During the lifetime of
/// Table both active and standby will always point to a valid initialized
/// location in memory.
///
/// Table cannot guarantee the thead safety of the tables. That requires the
/// Writer and Readers to synchronize in order to achieve. The way we achieve
/// this is:
/// 1. Only 1 Writer. Said Writer can only produce 1 WriteGuard, which is the
///    struct used to mutate 'standby_table'.
/// 2. Writer gains access to the 'standby_table' by calling Table::write.
///    Writer doesn't call Table::read.
/// 3. WriteGuard mutates 'standby_table' via the TableWriteGuard, which handles
///    swapping which table 'active_table' and 'standby_table' point to when
///    dropped. The tables are only swapped once for the lifetime of a given
///    WriteGuard.
/// 4. Any number of Readers can exist, which access 'active_table' via
///    Table::read. Readers never call Table::write.
/// 5. Readers signal to a Writer when they borrow 'active_table' and when they
///    stop borrow the table. This is how Writer knows when it is safe to mutate
///    'standby_table'.
///
/// All operations (mostly swap) on the active and standby tables are SeqCst,
/// since they need to be kept in sync relative to each other.
///
/// Consider adding comments about why not to do:
/// 1. AtomicPtr<(T, read_count)> - This means that the reader will grab the
///    active table before incrementing the counter, so it won't actually be
///    locked. Therefore a reader can grab the active table, then go to sleep
///    before incrementing the counter, and then have the writer do swapping and
///    only wake up while the writer holds the table.
/// 2. AtomicBool table0_active. I think the reason I didn't do this was to try
///    and keep reads as fast as possible. That involved another read + a branch
///    before grabbing the table; the current method is just grabbing a pointer.
///    I think it was possible to do this safely.
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

    // This is the only function that Readers should ever call to of Table.
    pub fn active_table(&self) -> &T {
        // Memory safety (valid pointer) is guaranteed by the class. See class
        // level comment.
        //
        // Thread safety isn't guaranteed by the compiler, instead our access
        // patterns (Reader & Writer) must enforce that this table is never
        // updated by the Writer so long as this read exists.
        unsafe { &*self.active_table.load(Ordering::SeqCst) }
    }

    /// Memory safety (valid pointer) is guaranteed by the class. See class
    /// level comment.
    ///
    /// This is thread safe as long as the user guarantees that:
    /// 1. The standby table is only accessed by the Writer.
    /// 2. Only 1 Writer will attempt to interact with the table at a time.
    /// 3. Writers wait for all Readers to switch off this table after calling
    ///    to `swap_active_and_standby`.
    pub fn standby_table(&self) -> &T {
        unsafe { &*self.standby_table.load(Ordering::SeqCst) }
    }

    /// Memory safety (valid pointer) is guaranteed by the class. See class
    /// level comment.
    ///
    /// This is thread safe as long as the user guarantees that:
    /// 1. The standby table is only accessed by the Writer.
    /// 2. Only 1 Writer will attempt to interact with the table at a time.
    /// 3. Writers wait for all Readers to switch off this table after calling
    ///    to `swap_active_and_standby`.
    pub fn standby_table_mut(&self) -> &mut T {
        unsafe { &mut *self.standby_table.load(Ordering::SeqCst) }
    }

    // When the WriteGuard is dropped, call to this function in order to swap
    // the active and standby tables. All futures ReadGuards will now be from
    // what was the standby_table which received all the updates during the
    // lifetime of TableWriteGuard. TableWriteGuard must be dropped after this
    // and only called after the Writer makes sure no ReadGuards are left
    // pointing to the new standby_table.
    pub fn swap_active_and_standby(&self) {
        let active_table = self.active_table.load(Ordering::SeqCst);
        let standby_table = self.standby_table.load(Ordering::SeqCst);
        assert_ne!(active_table, standby_table);

        // Swap the active and standby tables. These should never fail because
        // there can only ever be 1 writer which spawns only 1 write guard.
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

impl<T: fmt::Debug> fmt::Debug for Table<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table").finish()
    }
}
