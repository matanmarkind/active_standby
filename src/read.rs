use crate::table::Table;
use slab::Slab;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
        // Make sure to read 'first_epoch_after_update' first to guarantee it is never greater than epoch.
        std::sync::atomic::fence(Ordering::SeqCst);
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
    ///
    /// Performance: this function is blocking since we need to lock the set of
    /// readers.
    fn clone(&self) -> Reader<T> {
        // Safety: This is all done under the readers lock. Therefore there is
        // no race with the writer updating certain fields of the
        // SharedReaderInfo.
        let mut readers = self.readers.lock().unwrap();
        let info = ReaderEpochInfo {
            epoch: AtomicUsize::new(self.my_info.epoch.load(Ordering::Acquire)),
            first_epoch_after_update: AtomicUsize::new(
                self.my_info
                    .first_epoch_after_update
                    .load(Ordering::Acquire),
            ),
        };
        let key = readers.insert(Arc::new(info));

        Reader {
            readers: Arc::clone(&self.readers),
            my_key_in_readers: key,
            my_info: Arc::clone(&readers[key]),
            table: Arc::clone(&self.table),
        }
    }
}

impl<T> Reader<T> {
    pub fn new(readers: ReaderEpochInfos, table: Arc<Table<T>>) -> Reader<T> {
        let info = ReaderEpochInfo {
            epoch: AtomicUsize::new(0),
            first_epoch_after_update: AtomicUsize::new(0),
        };
        let key = readers.lock().unwrap().insert(Arc::new(info));

        Reader {
            my_info: Arc::clone(&readers.lock().unwrap()[key]),
            my_key_in_readers: key,
            readers: Arc::clone(&readers),
            table: Arc::clone(&table),
        }
    }

    pub fn read(&self) -> ReadGuard<'_, T> {
        let old_epoch = self.my_info.epoch.load(Ordering::Acquire);
        debug_assert_eq!(old_epoch % 2, 0);
        self.my_info.epoch.store(old_epoch + 1, Ordering::Release);

        // The reader must update the epoch before taking the table. This
        // effectively locks the active_table, making it safe for the reader to
        // proceed.
        std::sync::atomic::fence(Ordering::SeqCst);

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
