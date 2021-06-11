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
pub struct Table<T> {
    active_table: AtomicPtr<T>,
    standby_table: AtomicPtr<T>,
}

/// This is the struct used to gain mutable access to the standby_table. It
/// handles swapping the active and standby tables when dropped.
///
/// Note that unlike RwLockWriteGuard this class doesn't represent a unique lock
/// on the standby table. That is the responsibility of the Writer.
pub struct TableWriteGuard<'w, T> {
    active_table: &'w AtomicPtr<T>,
    standby_table: &'w AtomicPtr<T>,
}

impl<T> Table<T> {
    pub fn new(t: T) -> Table<T>
    where
        T: Clone,
    {
        Table {
            active_table: AtomicPtr::new(Box::into_raw(Box::new(t.clone()))),
            standby_table: AtomicPtr::new(Box::into_raw(Box::new(t))),
        }
    }

    pub fn read(&self) -> &'_ T {
        // Memory safety (valid pointer) is guaranteed by the class. See class
        // level comment.
        //
        // Thread safety isn't guaranteed by the compiler, instead our access
        // patterns (Reader & Writer) must enforce that this table is never
        // updated by the Writer so long as this read exists.
        unsafe { &*self.active_table.load(Ordering::SeqCst) }
    }

    pub fn write(&self) -> TableWriteGuard<'_, T> {
        TableWriteGuard {
            active_table: &self.active_table,
            standby_table: &self.standby_table,
        }
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

impl<T> TableWriteGuard<'_, T> {
    // When the WriteGuard is dropped, call to this function in order to swap
    // the active and standby tables. All futures ReadGuards will now be from
    // what was the standby_table which received all the updates during the
    // lifetime of TableWriteGuard. TableWriteGuard must be dropped after this
    // and only called after the Writer makes sure no ReadGuards are left
    // pointing to the new standby_table.
    pub fn swap_active_and_standby(&mut self) {
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

// TODO: consider adding "T: ?Sized" like std::sync::RwLock.
impl<'w, T> std::ops::Deref for TableWriteGuard<'w, T> {
    type Target = T;

    /// Memory safety (valid pointer) is guaranteed by the class. See class
    /// level comment.
    ///
    /// This is thread safe as long as the user guarantees that only 1
    /// TableWriteGuard can exist at a time.
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.standby_table.load(Ordering::SeqCst) }
    }
}

impl<'w, T> std::ops::DerefMut for TableWriteGuard<'w, T> {
    /// Memory safety (valid pointer) is guaranteed by the class. See class
    /// level comment.
    ///
    /// The thread safety of dereferencing this table is not guaranteed within
    /// this module because Table cannot guarantee that only 1 thread has a
    /// TableWriteGuard. Rather this requires keeping Table as private to the
    /// crate and ensuring the following properties:
    /// 1. We guarantee in 'await_standby_table_free' before WriteGuard creation
    ///    that there are no ReadGuards trying to read from this table.
    /// 2. There can only be 1 Writer for this table (no copy/clone interface).
    ///    There can only be 1 WriteGuard for this Writer (borrow checker should
    ///    enforce).
    /// 3. Readers will only switch to using this table when they are swapped on
    ///    TableWriteGuard::drop.
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.standby_table.load(Ordering::SeqCst) }
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for TableWriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableWriteGuard")
            .field("standby_table", unsafe {
                &*self.standby_table.load(Ordering::SeqCst)
            })
            .finish()
    }
}
