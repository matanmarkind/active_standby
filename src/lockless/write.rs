use crate::lockless::read::{Reader, ReaderEpochs};
use crate::lockless::table::Table;
use crate::types::*;
use slab::Slab;

/// InnerWriter is a simple struct which holds all of the pieces of info needed
/// by the Writer, it should have almost no logic.
struct InnerWriter<T> {
    // The underlying tables. This struct is responsible for returning the
    // correct active/standby table, and also for swapping them. This table
    // does not handle any synchronization across InnerWriter/Readers, rather
    // that is handled by the InnerWriter and Readers themselves.
    table: Arc<Table<T>>,

    // Information about each of the readers. Used by the Writer and Readers to
    // synchronize for thread safety; so that a Writer never mutates a table
    // that a ReadGuard is pointing to.
    readers: ReaderEpochs,

    // Log of operations to be performed on the second table.
    //
    // During a WriteGuard's lifetime, it mutates the stadnby table, but leaves
    // the active one constant for reads. These tables are then swapped when
    // the WriteGuard is dropped. Therefore, the next time a WriteGuard is
    // created, the standby table it points to will still need to have these
    // updates applied to it to keep the tables sychronized.
    ops_to_replay: Vec<Box<dyn FnOnce(&mut T) + Send>>,

    // A record of readers and their epoch after the most recent call to
    // table.swap_active_and_standby.
    //
    // Filled by the WriteGuard when it is dropped, and used by the Writer to
    // block creation of a new WriteGuard until there are no ReadGuards left
    // pointing to the standby table.
    //
    // {reader_key : first_epoch_after_swap}.
    blocking_readers: std::collections::HashMap<usize, usize>,
}

impl<T> std::fmt::Debug for InnerWriter<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // See comments on `Table::standby_table` for safety.
        let standby_table = unsafe { self.table.standby_table() };
        f.debug_struct("InnerWriter")
            .field("num_readers", &self.readers.lock().unwrap().len())
            .field("ops_to_replay", &self.ops_to_replay.len())
            .field("standby_table", standby_table)
            .finish()
    }
}

/// Writer class for mutating the underlying tables.
pub struct Writer<T> {
    inner: Mutex<InnerWriter<T>>,
}

impl<T> Writer<T> {
    pub fn from_identical(t1: T, t2: T) -> Writer<T> {
        // Create a InnerWriter object for handling active_standby tables.
        // - t1 & t2 are the two tables which will become the active and standby
        //   tables. They must be identical; this is left to the user to enforce.
        let inner = InnerWriter {
            table: Arc::new(Table::from_identical(t1, t2)),
            readers: Arc::new(Mutex::new(Slab::with_capacity(1024))),
            ops_to_replay: Vec::new(),
            blocking_readers: std::collections::HashMap::new(),
        };

        Writer {
            inner: Mutex::new(inner),
        }
    }

    /// Creates a new `Reader`.
    ///
    /// Returns None if the Mutex guarding the data is poisoned.
    pub fn new_reader(&self) -> Option<Reader<T>> {
        match self.inner.lock() {
            Ok(mg) => Some(Reader::new(&mg.readers, &mg.table)),
            Err(_) => None,
        }
    }

    /// Create a `WriteGuard` which is used to update the underlying tables.
    ///
    /// The function is responsible for waiting for the standby table to be be
    /// free for updates & for replaying the old ops from the last WriteGuard on
    /// it.
    ///
    /// Returns `PoisonError` if the Mutex guarding the data is poisoned.
    pub fn write(&self) -> LockResult<WriteGuard<'_, T>> {
        // Grab the mutex as the first thing.
        let mut mg = match self.inner.lock() {
            Ok(mg) => mg,
            Err(e) => {
                return Err(std::sync::PoisonError::new(WriteGuard {
                    guard: e.into_inner(),
                    swap_active_and_standby: false,
                }));
            }
        };

        // Wait until the standby table is free of ReadGuards so it is safe to
        // update.
        Writer::await_standby_table_free(&mut mg);
        std::sync::atomic::compiler_fence(Ordering::SeqCst);

        // Explicitly cast mg into the InnerWriter that it guards in order for
        // split borrowing to work. Without this line the compiler thinks that
        // the usage of table and ops_to_replay are conflicting mutable borrows
        // https://doc.rust-lang.org/nomicon/borrow-splitting.html
        let iw: &mut InnerWriter<T> = &mut mg;
        // See comments on `Table::standby_table_mut` for safety.
        let mut table = unsafe { iw.table.standby_table_mut() };

        // Replay all ops on the standby table.
        for op in iw.ops_to_replay.drain(..) {
            op(&mut table);
        }
        mg.ops_to_replay.clear();

        Ok(WriteGuard {
            guard: mg,
            swap_active_and_standby: true,
        })
    }

    /// Hangs until the standby table has no readers pointing to it, meaning it
    /// is safe for updating.
    fn await_standby_table_free(inner: &mut InnerWriter<T>) {
        // Wait until no reader is making use of the standby table.
        while !inner.blocking_readers.is_empty() {
            let readers = inner.readers.lock().unwrap();
            inner
                .blocking_readers
                .retain(|key, first_epoch_after_swap| {
                    let epoch = match readers.get(*key) {
                        None => {
                            // This Reader has been dropped.
                            return false;
                        }
                        Some(epoch) => epoch.load(Ordering::Acquire),
                    };

                    epoch <= *first_epoch_after_swap && *first_epoch_after_swap % 2 != 0
                });

            if !inner.blocking_readers.is_empty() {
                // Instead of just busy looping we will (potentially) yield this
                // thread and come back when the OS returns to us.
                spin_loop();
            }
        }
    }
}

impl<T> std::fmt::Debug for Writer<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Implemented this way (as oppsed to automatic) to avoid cluttering the
        // print statement with: "Writer : Mutex : InnerWriter: <info>".
        match self.inner.try_lock() {
            Ok(mg) => {
                // See comments on `Table::standby_table` for safety.
                let standby_table = unsafe { mg.table.standby_table() };
                f.debug_struct("Writer")
                    .field("num_readers", &mg.readers.lock().unwrap().len())
                    .field("ops_to_replay", &mg.ops_to_replay.len())
                    .field("standby_table", standby_table)
                    .finish()
            }
            Err(_) => self.inner.fmt(f),
        }
    }
}

/// Guard used for updating the tables.
///
/// Only 1 `WriteGuard` can ever exist at a time for a given table. `WriteGuard`
/// is responsible for updating the standby table, storing the updates for
/// replay on the other table, and swapping the active and standby tables on
/// drop.
pub struct WriteGuard<'w, T> {
    // Hold a MutexGuard to InnerWriter to guarantee no other WriteGuards exist
    // to this table and gain mutable access to the Tables. Due to issues with
    // self referential structs, we cannot also directly hold a &mut T to the
    // standby table. This results in all calls to the standby table requiring
    // an atomic load. Due to the nature of this struct being for mutations, and
    // the fact no other thread will touch this AtomicPtr due to there being
    // only 1 `WriteGuard`, it is expected that this shouldn't add too much
    // overhead.
    guard: MutexGuard<'w, InnerWriter<T>>,

    // If the table is poisoned we put the tables into lockdown and stop
    // swapping the active and standby tables.
    swap_active_and_standby: bool,
}

impl<'w, T> WriteGuard<'w, T> {
    /// Takes an update which will change the state of the underlying data. This
    /// is done through the interface of UpdateTables.
    ///
    /// Users should never use the return value to directly mutate the tables,
    /// since this will lead to them going out of sync.
    ///
    /// The update passed in must be valid for 'static because it will outlive
    /// the WriteGuard taking the update, so we can't make any limitations on
    /// it.
    pub fn update_tables<'a, R>(
        &'a mut self,
        mut update: impl UpdateTables<'a, T, R> + 'static + Sized + Send,
    ) -> R {
        // Explicitly grab the standby_table as a field of table, instead of via
        // a function call to `Table::standby_table_mut`. This is because we
        // need the lifetime of the table passed in to be tied to the lifetime
        // of the call to self.update_tables in order to allow return values
        // that have lifetimes (eg Vec::drain). If we call to standby_table_mut,
        // the lifetime of the table passed into `apply_first` is tied to the
        // returned reference (aka limited to this fn's scope), not self.
        //
        // See comments on `Table::standby_table_mut` for safety.
        let res = update
            .apply_first(unsafe { &mut *self.guard.table.standby_table.load(Ordering::SeqCst) });

        self.guard.ops_to_replay.push(Box::new(move |table| {
            update.apply_second(table);
        }));

        res
    }

    /// Like `update_tables` but allows the user to pass a closure for
    /// convenience. Only allows return values that own their data.
    ///
    /// TODO: Consider allowing return values that have lifetimes, this should
    /// likely be as safe as the explicit UpdateTables trait.
    pub fn update_tables_closure<R>(
        &mut self,
        update: impl Fn(&mut T) -> R + 'static + Sized + Send,
    ) -> R {
        // See comments on `Table::standby_table_mut` for safety.
        let res = update(unsafe { self.guard.table.standby_table_mut() });

        self.guard.ops_to_replay.push(Box::new(move |table| {
            update(table);
        }));

        res
    }

    // TODO: Consider adding an option to force_swap_active_and_standby. This
    // will probably need to come along with an option to force replay. If the
    // Mutex is poisoned we stop replaying updates and swapping the tables.
}

impl<'w, T> Drop for WriteGuard<'w, T> {
    fn drop(&mut self) {
        assert!(self.guard.blocking_readers.is_empty());

        if !self.swap_active_and_standby {
            // Should only be the case if the Mutex guarding InnerWriter was
            // poisoned. This means that the Active & Standby tables are locked,
            // so hopefully readers should be able to safely continue reading a
            // stale state.
            return;
        }

        // I initially implemented this as drop, and explicitly called
        // 'drop(table)'. This didn't actually take effect until the end
        // of this function though, causing us to record the epochs before the
        // swap had occurred. Caught by tsan.
        self.guard.table.swap_active_and_standby();

        // Make sure that swap occurs before recording the epoch.
        fence(Ordering::SeqCst);

        // Explicitly cast mg into the InnerWriter that it guards in order for
        // split borrowing to work. Without this line the compiler thinks that
        // the usage of readers and blocking_readers are conflicting mutable borrows
        // https://doc.rust-lang.org/nomicon/borrow-splitting.html
        let iw: &mut InnerWriter<T> = &mut self.guard;
        for (key, epoch) in iw.readers.lock().unwrap().iter_mut() {
            // Once the tables have been swapped, record the epoch of each
            // reader so that we will know if it is safe to update the new
            // standby table.
            let first_epoch_after_swap = epoch.load(Ordering::Acquire);
            if first_epoch_after_swap % 2 != 0 {
                // If the epoch is even, it means that there is no ReadGuard
                // active.
                iw.blocking_readers.insert(key, first_epoch_after_swap);
            }
        }
    }
}

impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // See comments on `Table::standby_table` for safety.
        unsafe { self.guard.table.standby_table() }
    }
}

impl<'w, T> std::fmt::Debug for WriteGuard<'w, T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // See comments on `Table::standby_table` for safety.
        let standby_table = unsafe { self.guard.table.standby_table() };
        f.debug_struct("WriteGuard")
            .field("swap_active_and_standby", &self.swap_active_and_standby)
            .field("num_readers", &self.guard.readers.lock().unwrap().len())
            .field("ops_to_replay", &self.guard.ops_to_replay.len())
            .field("standby_table", standby_table)
            .finish()
    }
}
