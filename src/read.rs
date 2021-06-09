use crate::table::Table;
use crate::types::*;
use slab::Slab;
use std::fmt;

/// Information that both Readers and writers need to read and update.
pub struct ReaderEpochInfo {
    // Epoch count of the reader, checked by the writer after updating table.
    // Updated by the reader, read by the writer.
    pub epoch: AtomicUsize,

    // First epoch value read by the writer after updating the table. Used to
    // determine when it is safe for the writer to begin mutating the standby
    // table. Only used by the writer.
    pub first_epoch_after_update: AtomicUsize,
}

impl Clone for ReaderEpochInfo {
    fn clone(&self) -> ReaderEpochInfo {
        let first_epoch_after_update = self.first_epoch_after_update.load(Ordering::Acquire);

        // Make sure to read 'first_epoch_after_update' first to guarantee it is
        // never greater than epoch. 
        fence(Ordering::SeqCst);

        let epoch = self.epoch.load(Ordering::Acquire);
        ReaderEpochInfo {
            epoch: AtomicUsize::new(epoch),
            first_epoch_after_update: AtomicUsize::new(first_epoch_after_update),
        }
    }
}
pub type ReaderEpochInfos = Arc<Mutex<Slab<Arc<ReaderEpochInfo>>>>;

/// Class used to obtain read guards to the underlying table. Obtaining a
/// ReadGuard should never suffer contention since the active table is promised
/// to never have a write guard.
pub struct Reader<T> {
    // Allows the Reader to generate new readers, and remove itself from the list on drop.
    readers: ReaderEpochInfos,

    // Key that references this reader, used on drop.
    my_key_in_readers: usize,

    // ReaderInfo of this Reader.
    my_info: Arc<ReaderEpochInfo>,

    // The table.
    table: Arc<Table<T>>,
}

pub struct ReadGuard<'r, T> {
    // Read by callers when dereferenceing the table.
    active_table: &'r T,

    // Incremented on Drop.
    epoch: &'r AtomicUsize,
}

impl<T> Clone for Reader<T> {
    /// Creates a new Reader that is independent of the initial one. All Readers
    /// should look identical to users.
    fn clone(&self) -> Reader<T> {
        Reader::new(&self.readers, &self.table)
    }
}

impl<T> Reader<T> {
    /// Create a new Reader.
    ///
    /// Performance: this function is potentially blocking since we need to lock
    /// the set of readers. This will compete with WriteGuard creation/deletion,
    /// but not during the lifetime of a WriteGuard.
    pub fn new(readers: &ReaderEpochInfos, table: &Arc<Table<T>>) -> Reader<T> {
        let info = ReaderEpochInfo {
            epoch: AtomicUsize::new(0),
            first_epoch_after_update: AtomicUsize::new(0),
        };
        let key = readers.lock().unwrap().insert(Arc::new(info));

        Reader {
            my_info: Arc::clone(&readers.lock().unwrap()[key]),
            my_key_in_readers: key,
            readers: Arc::clone(readers),
            table: Arc::clone(table),
        }
    }

    /// Obtain a read guard with which to inspect the active table.
    ///
    /// This is wait free since there is nothing to lock, and the Writer is
    /// responsible for never mutating the table that a Reader would want to
    /// read from.
    pub fn read(&mut self) -> ReadGuard<'_, T> {
        // Theoretically we could add a counter for number of entries and only
        // increment epoch on transitions from 0 <-> 1 guards. This would make
        // Reader re-entrant, which I tend to shy away from, since I think of it
        // as code smell.
        let old_epoch = self.my_info.epoch.load(Ordering::Acquire);
        debug_assert_eq!(old_epoch % 2, 0);
        self.my_info.epoch.store(old_epoch + 1, Ordering::Release);

        // The reader must update the epoch before taking the table. This
        // effectively locks the active_table, making it safe for the reader to
        // proceed knowing that the Writer will not be able to access this table
        // until epoch is implemented again.
        fence(Ordering::SeqCst);

        ReadGuard {
            active_table: self.table.read(),
            epoch: &self.my_info.epoch,
        }
    }
}

impl<T> Drop for Reader<T> {
    fn drop(&mut self) {
        self.readers.lock().unwrap().remove(self.my_key_in_readers);
    }
}

impl<T: fmt::Debug> fmt::Debug for Reader<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader").finish()
    }
}

impl<'r, T> Drop for ReadGuard<'r, T> {
    /// Update the epoch counter to notify the Writer that we are done using the
    /// 'active_table' and so is available for use as the new standby table.
    fn drop(&mut self) {
        let old_epoch = self.epoch.load(Ordering::Acquire);
        debug_assert_eq!(old_epoch % 2, 1);
        self.epoch.store(old_epoch + 1, Ordering::Release);
    }
}

impl<'r, T> std::ops::Deref for ReadGuard<'r, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.active_table
    }
}

impl<'r, T: fmt::Debug> fmt::Debug for ReadGuard<'r, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadGuard")
            .field("active_table", &self.active_table)
            .finish()
    }
}
