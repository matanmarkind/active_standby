use crate::read::Reader;
use crate::table::{Table, UpdateTables, UpdateTablesOpsList};
use crate::types:rRwLockReadGuard;
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

/// TODO: make writer a trait and have.
/// - VanillaWriter - current writer.
/// - SyncWriter - Add an Arc around the mutex and add clone.
/// - ChannelWriter - Will just be a transmit_queue around
///   channel<Vec<UpdateTables>> and an Arc<thread_handle>. There will be a busy
///   loop thread (mode Synchronous or
///   Asynchronous(SleepAfterFailureToAcquire)). This other thread promises that
///   all updates wil be applied, soon, as opposed to the other writers where if
///   there is a significant delay in between WriteGuard creation there can then
///   be a delay in updating the Reader.

/// Writer is the class used to control the underlying tables. It is neither
/// Send nor Sync. If you want multithreaded access to writing you must put it
/// behind a lock.
///
/// In order to interact with the underlying tables you must create a
/// WriteGuard.
///
/// Writer doesn't actually own the underlying data, so if Writer is Dropped,
/// this will not delete the tables. Instead they will only be dropped once all
/// Readers are also dropped.
pub struct Writer<T, U>
where
    U: UpdateTables<T>,
{
    table: Arc<Table<T>>,

    ops_list: UpdateTablesOpsList<T, U>,
}

impl<T, U> Writer<T, U>
where
    T: Clone,
    U: UpdateTables<T>,
{
    pub fn new(t: T) -> Writer<T, U> {
        Writer {
            table: Arc::new(Table::new(t)),
            ops_list: UpdateTablesOpsList::new(),
        }
    }
}

impl<T, U> Writer<T, U>
where
    U: UpdateTables<T>,
{
    /// Create a WriteGuard to allow users to update the the data. There will
    /// only be 1 WriteGuard at a time.
    ///
    /// This function may be slow because:
    /// 1. Lock contention on the standby_table. This can occur if a ReadGuard
    ///    which was created before the last WriteGuard was dropped, still has
    ///    not itself been dropped.
    /// 2. Replaying all of the updates that were applied to the last
    ///    WriteGuard.
    pub fn write(&mut self) -> WriteGuard<'_, T, U> {
        WriteGuard::new(self)
    }

    /// Creates a new Reader which points to the data held in Writer.
    pub fn new_reader(&self) -> Reader<T> {
        Reader::new(Arc::clone(&self.table))
    }

    // While it is thread safe to create other Writers, it would likely lead to
    // unsafe usage. This is because of the delay in updates. If I had 2 threads
    // updating the tables, they may check the state of the table, then make a
    // decision about what update to send, but the other thread may commit its
    // updates first, thereby causing the table to be in a different state than
    // expected when the commit occurs.
}

/// I am not certain if I really need to require that U be send. Especially if a
/// user is passing an empty writer to another thread I don't think this is
/// needed.
unsafe impl<T, U: UpdateTables<T> + Send> Send for Writer<T, U> {}

impl<T, U> Drop for Writer<T, U>
where
    U: UpdateTables<T>,
{
    /// Publish the updates to the Readers. This is done synchronously, so Drop
    /// will hang until it can obtain a RwLockWriteGuard of the standby_table.
    fn drop(&mut self) {
        // We rely on knowing that this is the only Writer and it can only call
        // to 'write' when there are no existing WriteGuards.
        let table = unsafe {
            std::mem::transmute::<*const Table<T>, &mut Table<T>>(Arc::as_ptr(&self.table))
        };

        table.publish_updates(&mut self.ops_list);
    }
}

impl<T: fmt::Debug, U> fmt::Debug for Writer<T, U>
where
    U: UpdateTables<T>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("op_list", &self.ops_list)
            .finish()
    }
}

/// WriteGuard is the way to mutate the underlying tables. A Writer can only
/// generate 1 at a time, which is enforced by the borrow checker on creation.
///
/// Unlike an RwLockWriteGuard, we don't mutate the underlying data in a
/// transparent manner. Instead the caller must pass in a function which
/// implements the UpdateTables trait to mutate the underlying data.
///
/// When dereferencing a WriteGuard we see the state of the standby_table, not
/// the active_table which the Readers dereference.
///
/// Upon Drop, a WriteGuard automatically publishes the changes to the Readers,
/// by swapping the active and standby tables. The updates are only performed on
/// the new standby table the next time a WriteGuard is created. This is to
/// minimize thread contention. That way Readers will have a chance to switch to
/// reading from the new active table before trying to WriteLock the new standby
/// table.
pub struct WriteGuard<'w, T, U>
where
    U: UpdateTables<T>,
{
    // Record the ops that were applied to standby_table to be replayed the next
    // time we create a WriteGuard.
    writer: &'w mut Writer<T, U>,

    /// WriteGuard must remain in the same thread as the Writer that created it.
    /// TODO: double check if this is necessary.
    _unimpl_send: PhantomData<*const T>,
}

/// WriteGuard is likely to be the trickiest for use. It is critical that the
/// user make sure that any mutation that occurs on one table, also occurs on
/// the other. In order to achieve this we provider 3 interfaces:
/// - update_tables - the simplest and safest interface. Takes in a single
///   function and applies it to both tables.
/// - update_tables_individually - allows for more complex return values
///   specifically with lifetime requirements.
/// - standby_table_and_active_table_enqueue - the least preffered interface due
///   to expectations of misuse. The user must make sure that all mutations
///   performed on the standby_table directly are reflected in the update
///   operation(s) they enqueue.
impl<'w, T, U> WriteGuard<'w, T, U>
where
    U: UpdateTables<T>,
{
    /// If the standby_table is free (no readers), publish all of the updates to
    /// it.
    fn try_to_publish(&mut self) {
        // We rely on knowing that this is the only Writer and it can only call
        // to 'write' when there are no existing WriteGuards.
        let table = unsafe {
            std::mem::transmute::<*const Table<T>, &mut Table<T>>(Arc::as_ptr(&self.writer.table))
        };
        table.try_to_publish_updates(&mut self.writer.ops_list);
    }
    fn apply_updates_to_standby_table(&mut self) {
        // We rely on knowing that this is the only Writer and it can only call
        // to 'write' when there are no existing WriteGuards.
        let table = unsafe {
            std::mem::transmute::<*const Table<T>, &mut Table<T>>(Arc::as_ptr(&self.writer.table))
        };
        table.apply_updates_to_standby_table(&mut self.writer.ops_list);
    }

    fn new(writer: &mut Writer<T, U>) -> WriteGuard<'_, T, U> {
        let mut wg = WriteGuard {
            writer,
            _unimpl_send: PhantomData,
        };
        wg.mut_table()
            .try_to_publish_updates(&mut wg.writer.ops_list);
        wg
    }

    /// Passes in a function to mutate the tables that is performed on both
    /// tables. The operation is applied synchronously on the standby_table and
    /// the return value is returned to the caller. The op is then enqueued and
    /// will be called on the current active_table before the next WriteGuard is
    /// created (when it will be the standby_table).
    ///
    /// Please be aware that any mutations that the caller makes on a returned
    /// value that affect the underlying table will not be reflected when the
    /// tables swap since we only replay the function, we don't know what the
    /// caller will do with it.
    pub fn update_tables(&mut self, update: U) -> &mut Self {
        self.writer.ops_list.push(update);
        self
    }

    /// Perform all enqueued updates on the standby_table, but do not publish
    /// the changes to the Readers (aka standby and active tables don't change
    /// roles).
    ///
    /// This may hang until WriteGuard is able to obtain a write lock on the
    /// standby table.
    pub fn apply_updates_to_standby_table(&mut self) -> &Self {
        self.mut_table()
            .apply_updates_to_standby_table(&mut self.writer.ops_list);
        self
    }

    pub fn read(&mut self) -> RwLockReadGuard<'_, T> {
        self.writer.table.standby_table_guard()
    }
}

/// When the WriteGuard is dropped we swap the active and standby tables. We
/// don't update the new standby table until a new WriteGuard is created.
impl<'w, T, U> Drop for WriteGuard<'w, T, U>
where
    U: UpdateTables<T>,
{
    fn drop(&mut self) {
        self.mut_table()
            .try_to_publish_updates(&mut self.writer.ops_list);
    }
}

impl<'w, T, U> fmt::Debug for WriteGuard<'w, T, U>
where
    T: fmt::Debug,
    U: UpdateTables<T>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteGuard")
            .field("writer", &self.writer)
            .finish()
    }
}

/// Writer which is Send + Sync by just wrapping a Writer in a Mutex.
///
/// Mostly here to save users having to add "impl Sync" if they want to use
/// Mutex<Writer>.
pub struct SyncWriter<T, U>
where
    U: UpdateTables<T> + Send,
{
    writer: Mutex<Writer<T, U>>,
}
unsafe impl<T, U: UpdateTables<T> + Send> Sync for SyncWriter<T, U> {}

impl<T, U> SyncWriter<T, U>
where
    T: Clone,
    U: UpdateTables<T> + Send,
{
    pub fn new(t: T) -> SyncWriter<T, U> {
        SyncWriter {
            writer: Mutex::new(Writer::new(t)),
        }
    }
}

impl<T, U> std::ops::Deref for SyncWriter<T, U>
where
    U: UpdateTables<T> + Send,
{
    type Target = Mutex<Writer<T, U>>;
    fn deref(&self) -> &Self::Target {
        &self.writer
    }
}

impl<T, U> std::ops::DerefMut for SyncWriter<T, U>
where
    U: UpdateTables<T> + Send,
{
    fn deref_mut(&mut self) -> &mut Mutex<Writer<T, U>> {
        &mut self.writer
    }
}

impl<T, U> fmt::Debug for SyncWriter<T, U>
where
    T: fmt::Debug,
    U: UpdateTables<T> + Send,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncWriter")
            .field("writer", &self.writer)
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::thread;

    enum UpdateVecOp<T> {
        Push(T),
        Pop,
    }
    struct UpdateVec<T> {
        update_op: UpdateVecOp<T>,
    }
    impl<T> UpdateTables<Vec<T>> for UpdateVec<T>
    where
        T: Clone,
    {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            match &self.update_op {
                UpdateVecOp::Push(element) => {
                    table.push(element.clone());
                }
                UpdateVecOp::Pop => {
                    table.pop();
                }
            }
        }

        fn apply_second(self: Box<Self>, table: &mut Vec<T>) {
            match self.update_op {
                UpdateVecOp::Push(element) => {
                    table.push(element);
                }
                UpdateVecOp::Pop => {
                    table.pop();
                }
            }
        }
    }

    type VecWriter<T> = Writer<Vec<T>, UpdateVec<T>>;

    #[test]
    fn one_guard() {
        let mut writer = VecWriter::<i32>::new(vec![]);
        let _wg = writer.write();

        // If we uncomment this line the program fails to compile due to a
        // second mutable borrow. This is what we want to guarantee there can
        // only be 1 WriteGuard at a time.
        //
        // let wg2 = writer.write();
    }

    #[test]
    fn publish_update() {
        let mut writer = VecWriter::<i32>::new(vec![]);
        let reader = writer.new_reader();
        assert_eq!(reader.read().len(), 0);

        {
            let mut wg = writer.write();
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(2),
            });
            assert_eq!(*reader.read(), vec![]);
        }
        assert_eq!(*reader.read(), vec![2]);
    }

    #[test]
    fn multi_apply() {
        // As opposed to the above which could mask an issue of just applying
        // the last update, show multiple updates with their side effects.
        let mut writer = VecWriter::<i32>::new(vec![]);
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(2),
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(3),
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(4),
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Pop,
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(5),
            });
        }

        assert_eq!(*reader.read(), vec![2, 3, 5]);
    }

    #[test]
    fn multi_publish() {
        // As opposed to the above which could mask an issue of just applying
        // the last update, show multiple updates with their side effects.
        let mut writer = VecWriter::<i32>::new(vec![]);
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(2),
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(3),
            });
        }
        assert_eq!(*reader.read(), vec![2, 3]);

        {
            let mut wg = writer.write();
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(4),
            });
        }
        assert_eq!(*reader.read(), vec![2, 3, 4]);

        {
            let mut wg = writer.write();
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Pop,
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(5),
            });
        }

        assert_eq!(*reader.read(), vec![2, 3, 5]);
    }

    #[test]
    fn delayed_publish() {
        // If a
        let mut writer = VecWriter::<i32>::new(vec![]);
        let reader = writer.new_reader();
        {
            let _rg = reader.read();
            {
                // Create a WriteGuard. This applies no ops, since none have
                // been enqueued, and swaps the tables. '_rg' now holds the
                // standby table, so no updates will be applied until it is
                // dropped.
                let mut wg = writer.write();
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Push(2),
                });
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Push(3),
                });
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Push(4),
                });
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Pop,
                });
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Push(5),
                });
            }
            assert_eq!(*reader.read(), vec![]);
            // '_rg' will now be dropped leaving no locks on the standby_table.
        }
        assert_eq!(*reader.read(), vec![]);

        {
            // This is the first time a WriteGuard is created/dropped when there
            // are no readers holding standby_table.
            let _wg = writer.write();
            assert_eq!(*reader.read(), vec![2, 3, 5]);
        }
        assert_eq!(*reader.read(), vec![2, 3, 5]);
    }

    #[test]
    fn multi_thread() {
        let mut writer = VecWriter::<i32>::new(vec![]);
        let reader = writer.new_reader();
        let handler = thread::spawn(move || {
            while *reader.read() != vec![2, 3, 5] {
                // Since commits oly happen when a WriteGuard is dropped no reader
                // will see this state.
                assert_ne!(*reader.read(), vec![2, 3, 4]);
            }

            // Show multiple readers in multiple threads.
            let reader2 = Reader::clone(&reader);
            let handler = thread::spawn(move || while *reader2.read() != vec![2, 3, 5] {});
            assert!(handler.join().is_ok());
        });

        {
            let mut wg = writer.write();
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(2),
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(3),
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(4),
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Pop,
            });
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(5),
            });
        }

        assert!(handler.join().is_ok());
    }

    #[test]
    fn writer_dropped() {
        // Show that when the Writer is dropped, Readers remain valid.
        let reader;
        {
            let mut writer = VecWriter::<i32>::new(vec![]);
            reader = writer.new_reader();

            {
                let mut wg = writer.write();
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Push(2),
                });
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Push(3),
                });
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Push(4),
                });
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Pop,
                });
                wg.update_tables(UpdateVec {
                    update_op: UpdateVecOp::Push(5),
                });
            }
        }
        assert_eq!(*reader.read(), vec![2, 3, 5]);
    }

    #[test]
    fn debug_str() {
        let mut writer = VecWriter::<i32>::new(vec![]);
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            wg.update_tables(UpdateVec {
                update_op: UpdateVecOp::Push(2),
            });
            assert_eq!(
                format!("{:?}", wg),
                "WriteGuard { writer: Writer { num_ops_to_apply: 1 } }"
            );
        }
        assert_eq!(format!("{:?}", writer), "Writer { num_ops_to_apply: 1 }");
        assert_eq!(
            format!("{:?}", reader.read()),
            "RwLockReadGuard { lock: RwLock { data: [2] } }"
        );
        assert_eq!(
            format!("{:?}", reader),
            "Reader { read_guard: RwLockReadGuard { lock: RwLock { data: [2] } } }"
        );

        let sync_writer = SyncWriter::<Vec<i32>, UpdateVec<i32>>::new(vec![]);
        assert_eq!(
            format!("{:?}", sync_writer),
            "SyncWriter { writer: Mutex { data: Writer { num_ops_to_apply: 0 } } }"
        );
    }
}
