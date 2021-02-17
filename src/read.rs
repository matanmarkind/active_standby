use crate::table::Table;
use crate::types::RwLockReadGuard;
use std::fmt;
use std::sync::Arc;

pub struct Reader<T> {
    table: Arc<Table<T>>,
}

pub type ReadGuard<'r, T> = RwLockReadGuard<'r, T>;

/// Reader is the class used to read from an active_standby table. It's use is
/// just like reading from an RwLock, but with the promise that there is never
/// contention with a Writer.
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
        self.table.read_guard()
    }
}

impl<T: fmt::Debug> fmt::Debug for Reader<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("read_guard", &self.read())
            .finish()
    }
}
