use crate::table::Table;
use std::sync::{Arc, RwLockReadGuard};

pub struct Reader<T> {
    table: Arc<Table<T>>,
}

impl<T> Reader<T> {
    pub fn clone(orig: &Reader<T>) -> Reader<T> {
        Reader {
            table: Arc::clone(&orig.table),
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        self.table.read_guard()
    }
}