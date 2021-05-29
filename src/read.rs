use crate::table::{RwLockReadGuard, Table};
use std::fmt;
use std::sync::Arc;

/// Class used to obtain read guards to the underlying table. Obtaining a
/// ReadGuard should never suffer contention since the active table is promised
/// to never have a write guard.
pub struct Reader<T> {
    table: Arc<Table<T>>,
}

pub type ReadGuard<'r, T> = RwLockReadGuard<'r, T>;

impl<T> Reader<T> {
    // This is effectively a crate private function since Table is a private
    // type.
    pub fn new(table: Arc<Table<T>>) -> Reader<T> {
        Reader { table }
    }

    pub fn clone(orig: &Reader<T>) -> Reader<T> {
        Reader {
            table: Arc::clone(&orig.table),
        }
    }

    pub fn read(&self) -> ReadGuard<'_, T> {
        self.table.read()
    }
}

impl<T: fmt::Debug> fmt::Debug for Reader<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("read", &self.read())
            .finish()
    }
}
