use crate::table::Table;
use crate::types::RwLockReadGuard;
use std::sync::Arc;

pub struct Reader<T> {
    table: Arc<Table<T>>,
}

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

    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        self.table.read_guard()
    }
}
