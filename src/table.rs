use std::fmt;
use std::sync::atomic::{AtomicPtr, Ordering};

// Struct which handled write locking the table. Meant to look identical to the
// standard TableWriteGuard, except that internally it makes sure to swap the
// active and standby tables when dropped.
//
// TODO: consider adding T: ?Sized + 'a like std::sync::TableWriteGuard.
pub struct TableWriteGuard<'w, T> {
    active_table: &'w AtomicPtr<T>,
    standby_table: &'w AtomicPtr<T>,
}

/// When the TableWriteGuard is dropped we swap the active and standby tables. We
/// don't update the new standby table until a new TableWriteGuard is created.
impl<T> Drop for TableWriteGuard<'_, T> {
    fn drop(&mut self) {
        let active_table = self.active_table.load(Ordering::Acquire);
        let standby_table = self.standby_table.load(Ordering::Acquire);
        assert_ne!(active_table, standby_table);

        // Swap the active and standby tables. These should never fail because
        // there can only ever be 1 writer which spawns only 1 write guard.
        let res = self.active_table.compare_exchange(
            active_table,
            standby_table,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        assert_eq!(res, Ok(active_table));

        let res = self.standby_table.compare_exchange(
            standby_table,
            active_table,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        assert_eq!(res, Ok(standby_table));
    }
}

// TODO: double check that compile time enforces only 1 Writer, WriteGuard,
// TableWriteGuard.
impl<'w, T> std::ops::Deref for TableWriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // It is safe to dereference this pointer because it cannot be null or
        // uninitialized. TableWriteGuard is tied to the lifetime of Table.
        // Table initializes this pointer on construction and only drops it when
        // Table itself is dropped. Since we guarantee that for the lifetime of
        // Table this pointer must be valid, it therefore must be valid for the
        // lifetime of TableWriteGuard.
        unsafe { &*self.standby_table.load(Ordering::Acquire) }
    }
}

impl<'w, T> std::ops::DerefMut for TableWriteGuard<'w, T> {
    fn deref_mut(&mut self) -> &mut T {
        // It is safe to dereference this pointer because it cannot be null or
        // uninitialized. TableWriteGuard is tied to the lifetime of Table.
        // Table initializes this pointer on construction and only drops it when
        // Table itself is dropped. Since we guarantee that for the lifetime of
        // Table this pointer must be valid, it therefore must be valid for the
        // lifetime of TableWriteGuard.
        unsafe { &mut *self.standby_table.load(Ordering::Acquire) }
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for TableWriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableWriteGuard").finish()
    }
}

// Table is not safe, we rely entirely on the Writer and Reader implementations
// to guarantee thread safety.
pub struct Table<T> {
    active_table: AtomicPtr<T>,
    standby_table: AtomicPtr<T>,
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
        unsafe { &*self.active_table.load(Ordering::Acquire) }
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
        // It is safe to dereference this pointer because it cannot be null or
        // uninitialized. Table initializes these pointers on construction and
        // only drops them here.
        unsafe {
            let _active_table = Box::from_raw(self.active_table.load(Ordering::Acquire));
            let _standby_table = Box::from_raw(self.standby_table.load(Ordering::Acquire));
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Table<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table").finish()
    }
}

/// This marks that Table is Sync, since it handles its own synchronization. It
/// is also important to mark it this way so that Writer can be Send without T
/// needing to be Sync.
///
/// TODO: I don't think this is true now.
unsafe impl<T> Sync for Table<T> where T: Send {}
