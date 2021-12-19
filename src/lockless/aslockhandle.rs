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

        // Getting a Reader at this point should be guaranteed to work since the
        // Mutex within Writer has never been locked and therefore cannot be
        // poisoned.
        let reader = writer.new_reader().unwrap();

        AsLockHandle {
            writer: std::sync::Arc::new(writer),
            reader,
        }
    }

    pub fn read(&self) -> ReadGuard<'_, T> {
        self.reader.read()
    }
}

impl<T> AsLockHandle<T> {
    pub fn write(&self) -> LockResult<WriteGuard<'_, T>> {
        self.writer.write()
    }
}

impl<T> Clone for AsLockHandle<T> {
    fn clone(&self) -> AsLockHandle<T> {
        let writer = std::sync::Arc::clone(&self.writer);
        let reader = self.reader.clone();
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
