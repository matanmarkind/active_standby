use crate::lockless::read::{ReadGuard, Reader};
use crate::lockless::write::{WriteGuard, Writer};
use crate::types::*;

pub struct AsLockHandle<T> {
    writer: std::sync::Arc<Writer<T>>,
    reader: Reader<T>,
}

impl<T> AsLockHandle<T> {
    pub fn from_identical(t1: T, t2: T) -> AsLockHandle<T> {
        let writer = Writer::from_identical(t1, t2);
        let reader = writer.new_reader();
        AsLockHandle {
            writer: std::sync::Arc::new(writer),
            reader,
        }
    }

    pub fn read(&self) -> ReadGuard<'_, T> {
        self.reader.read()
    }
}

#[cfg(active_standby_compare_tables_equal)]
impl<T> AsLockHandle<T>
where
    T: PartialEq + std::fmt::Debug,
{
    pub fn write(&self) -> LockResult<WriteGuard<'_, T>> {
        let wg = self.writer.write()?;
        if (*wg != *self.read()) {
            return Err(PoisonError::new(wg));
        }
        Ok(wg)
    }
}

#[cfg(not(active_standby_compare_tables_equal))]
impl<T> AsLockHandle<T> {
    pub fn write(&self) -> LockResult<WriteGuard<'_, T>> {
        self.writer.write()
    }
}

impl<T> Clone for AsLockHandle<T> {
    fn clone(&self) -> AsLockHandle<T> {
        let writer = std::sync::Arc::clone(&self.writer);
        let reader = writer.new_reader();
        AsLockHandle { writer, reader }
    }
}

impl<T> std::fmt::Debug for AsLockHandle<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsLockHandle")
            .field("writer", &self.writer)
            .field("reader", &self.reader)
            .finish()
    }
}
