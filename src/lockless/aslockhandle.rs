use crate::lockless::read::{ReadGuard, Reader};
use crate::lockless::write::{WriteGuard, Writer};
use crate::types::*;

pub struct AsLockHandle<T> {
    writer: std::sync::Arc<Writer<T>>,
    reader: Reader<T>,
}

impl<T> AsLockHandle<T> {
    // TODO: Add specialization of from_identical which compares t1 & t2.
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

    pub fn read(&self) -> LockResult<ReadGuard<'_, T>> {
        Ok(self.reader.read())
    }

    pub fn write(&self) -> LockResult<WriteGuard<'_, T>> {
        self.writer.write()
    }
}

impl<T> AsLockHandle<T>
where
    T: Clone,
{
    pub fn new(t: T) -> AsLockHandle<T> {
        Self::from_identical(t.clone(), t)
    }
}

impl<T> AsLockHandle<T>
where
    T: Default,
{
    pub fn default() -> AsLockHandle<T> {
        Self::from_identical(T::default(), T::default())
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
