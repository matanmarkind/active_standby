use crate::lockless::read::{ReadGuard, Reader};
use crate::lockless::write::{WriteGuard, Writer};
use crate::types::*;

/// Public primitive for building lockess active_standby data structures. Give
/// users both read and write access to the tables.
///
/// Write interaction is done via the `update_tables` interface, instead of the
/// user gaining direct access to the underlying tables. This is because there
/// are truly 2 tables (active/standby) under the hood, and both need to be kept
/// in sync. This is handled by accepting updates, which AsLockHandle applies on
/// both tables.
///
/// It is also worth noting that this data structure should not be shared across
/// threads/tasks. Rather think of it as closer to a parallel of Arc<RwLock>
/// than a plan RwLock. Meaning that each thread/task should get its own
/// AsLockHandle (via clone).
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

    /// Obtain a read guard with which to inspect the active table.
    ///
    /// This is wait free since there is nothing to lock, and the Writer is
    /// responsible for never mutating the table that a ReadGuard points to.
    pub fn read(&self) -> LockResult<ReadGuard<'_, T>> {
        Ok(self.reader.read())
    }

    /// Create a `WriteGuard` which is used to update the underlying tables.
    ///
    /// The function is responsible for waiting for the standby table to be be
    /// free for updates & for replaying the old ops from the last WriteGuard on
    /// it.
    ///
    /// Returns `PoisonError` if the Mutex guarding the data is poisoned.
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
