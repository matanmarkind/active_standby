use crate::types::*;
use std::fmt;
use std::mem::ManuallyDrop;

/// Struct for holding tables that can be interacted with like an RwLock,
/// including being shared across threads/tasks via Arc (as opposed to the
/// lockless version which requires independent copies per task).
pub struct AsLock<T> {
    // The underlying tables. This struct is responsible for returning the
    // correct active/standby table.
    active_table: AtomicPtr<RwLock<T>>,
    standby_table: AtomicPtr<RwLock<T>>,

    /// Log of operations to be performed on the second table. This gets played
    /// on the standby table when creating a WriteGuard, as opposed to when
    /// dropping it, to minimize lock contention. This is in the hopes that by
    /// waiting until the next time a `WriteGuard` is created, we give the readers
    /// time to switch to reading from the new `active_table`.
    ///
    /// This mutex is used to guarantee that 'write' is single threaded, and so
    /// locking it must be done before calling to `standby_table.write`.
    ops_to_replay: Mutex<Vec<Box<dyn FnOnce(&mut T) + Send>>>,
}

/// Guard used for updating the tables.
pub struct WriteGuard<'w, T> {
    // Pointers to the tables. Used to swap them on drop.
    active_table: &'w AtomicPtr<RwLock<T>>,
    standby_table: &'w AtomicPtr<RwLock<T>>,

    // Guard used to mutably access `standby_table` without constantly having to
    // load the atomic pointer and write lock the table. This is wrapped inside
    // of `ManuallyDrop` to guarantee that the table is unlocked before swapping
    // the active and standby tables. This is needed to gurantee that reads are
    // never blocked.
    //
    // This introduces the risk of a deadlock though is we forget to release
    // `guard`.
    guard: ManuallyDrop<RwLockWriteGuard<'w, T>>,

    // Hold onto updates for replay when the next WriteGuard is created.
    ops_to_replay: MutexGuard<'w, Vec<Box<dyn FnOnce(&mut T) + Send>>>,

    // If the table is poisoned we put the tables into lockdown and stop
    // swapping the active and standby tables.
    swap_active_and_standby: bool,
}

// Define ReadGuard locally so that the type names are consistent; across
// lockless & shared, as well as internally (WriteGuard & RwLockReadGuard seem
// unwieldy).
pub type ReadGuard<'r, T> = RwLockReadGuard<'r, T>;

impl<T> AsLock<T> {
    /// Create an AsLock object for handling active_standby tables.
    /// - t1 & t2 are the two tables which will become the active and standby
    ///   tables. They must be identical; this is left to the user to enforce.
    pub fn from_identical(t1: T, t2: T) -> AsLock<T> {
        AsLock {
            active_table: AtomicPtr::new(Box::into_raw(Box::new(RwLock::new(t1)))),
            standby_table: AtomicPtr::new(Box::into_raw(Box::new(RwLock::new(t2)))),
            ops_to_replay: Mutex::default(),
        }
    }

    pub fn read(&self) -> LockResult<ReadGuard<'_, T>> {
        // SAFETY: The only safety issue here is active_table being an invalid
        // ptr. This should never happen since standby/active table are created
        // on creation and only droppe dhwne AsLock is dropped. In between they
        // are swapped, but that shouldn't affect their valididty as pointers.
        unsafe { &*self.active_table.load(Ordering::SeqCst) }.read()
    }

    /// Create a WriteGuard to allow users to update the the data. There will
    /// only be 1 WriteGuard at a time.
    ///
    /// This function may be slow because:
    /// 1. Another WriteGuard exists. In practice this means lock contention on
    ///    `ops_to_replay`.
    /// 2. A ReadGuard still points to the standby table, meaning that this
    ///    ReadGuard came into existence before the last WriteGuard was dropped.
    /// 3. Replaying all of the updates that were applied to the last
    ///    WriteGuard.
    pub fn write(&self) -> LockResult<WriteGuard<'_, T>> {
        // Done first to ensure that it is single threaded. If WriteGuard is
        // ever poisoned, this will make all future calls to `write` panic.
        //
        // The only way for the tables to get poisoned is via an
        // RwLockWriteGuard (RwLockReadGuard won't poison if dropped via a
        // panic). Therefore we shouldn't need to handle `wg` (below) being
        // `Err`, since in any situation where the WriteGuard would be poisoned,
        // this mutex would also be poisoned, preventing that code from running.
        let mut ops_to_replay = self.ops_to_replay.lock().unwrap();

        // Grab the standby table and obtain a WriteGuard to it. This may hang
        // on old ReadGuards (aka those that exist from before the last
        // WriteGuard was dropped).
        //
        // SAFETY: The only safety issue here is standby_table being an invalid
        // ptr. This should never happen since standby/active table are created
        // on creation and only dropped when AsLock is dropped. In between they
        // are swapped, but that shouldn't affect their valididty as pointers.
        let wg = unsafe { &*self.standby_table.load(Ordering::SeqCst) }.write();
        let mut wg = match wg {
            Ok(wg) => wg,
            Err(e) => {
                return Err(std::sync::PoisonError::new(WriteGuard {
                    guard: ManuallyDrop::new(e.into_inner()),
                    active_table: &self.active_table,
                    standby_table: &self.standby_table,
                    ops_to_replay: ops_to_replay,
                    swap_active_and_standby: false,
                }));
            }
        };

        // Replay all ops on the standby table.
        for op in ops_to_replay.drain(..) {
            op(&mut wg);
        }
        ops_to_replay.clear();

        Ok(WriteGuard {
            guard: ManuallyDrop::new(wg),
            active_table: &self.active_table,
            standby_table: &self.standby_table,
            ops_to_replay: ops_to_replay,
            swap_active_and_standby: true,
        })
    }
}

impl<T> Drop for AsLock<T> {
    fn drop(&mut self) {
        // Memory safety (valid pointer) is guaranteed by the class. See class
        // level comment.
        unsafe {
            let _active_table = Box::from_raw(self.active_table.load(Ordering::SeqCst));
            let _standby_table = Box::from_raw(self.standby_table.load(Ordering::SeqCst));
        }
    }
}

impl<T> AsLock<T>
where
    T: Clone,
{
    pub fn new(t: T) -> AsLock<T> {
        Self::from_identical(t.clone(), t)
    }
}

impl<T> AsLock<T>
where
    T: Default,
{
    pub fn default() -> AsLock<T> {
        Self::from_identical(T::default(), T::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for AsLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let num_ops_to_replay = self.ops_to_replay.lock().unwrap().len();
        f.debug_struct("AsLock")
            .field("num_ops_to_replay", &num_ops_to_replay)
            .field("active_table", &*self.read().unwrap())
            .finish()
    }
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
        // SAFETY: We can guarantee that self.guard is valid, because the value
        // is only ever set on WriteGuard creation from a valid value and is
        // never changed.
        let res = update.apply_first(&mut self.guard);

        self.ops_to_replay.push(Box::new(move |table| {
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
        // SAFETY: We can guarantee that self.guard is valid, because the value
        // is only ever set on WriteGuard creation from a valid value and is
        // never changed.
        let res = update(&mut self.guard);
        self.ops_to_replay.push(Box::new(move |table| {
            update(table);
        }));
        res
    }
}

impl<'w, T> Drop for WriteGuard<'w, T> {
    fn drop(&mut self) {
        {
            // SAFETY: We must guarantee that all calls to WriteGuard::drop drop
            // self.guard. We can guarantee that it is valid, because the value
            // is only ever set on WriteGuard creation from a valid value and is
            // never changed.
            unsafe { ManuallyDrop::drop(&mut self.guard) };
        }

        if !self.swap_active_and_standby {
            // Should only be the case if the Mutex guarding InnerWriter was
            // poisoned. This means that the Active & Standby tables are locked,
            // so hopefully readers should be able to safely continue reading a
            // stale state.
            return;
        }

        // Swap the tables.
        let active_table = self.active_table.load(Ordering::SeqCst);
        let standby_table = self.standby_table.load(Ordering::SeqCst);
        assert_ne!(active_table, standby_table);

        // Swap the active and standby tables. These should never fail because
        // there can only ever be 1 writer which spawns only 1 WriteGuard.
        let res = self.active_table.compare_exchange(
            active_table,
            standby_table,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        assert_eq!(res, Ok(active_table));

        let res = self.standby_table.compare_exchange(
            standby_table,
            active_table,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        assert_eq!(res, Ok(standby_table));
    }
}

impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &**self.guard
    }
}

impl<'w, T: fmt::Debug> fmt::Debug for WriteGuard<'w, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use std::ops::Deref;
        f.debug_struct("WriteGuard")
            .field("num_ops_to_replay", &self.ops_to_replay.len())
            .field("standby_table", self.deref())
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::thread;

    struct PushVec<T> {
        value: T,
    }
    impl<'a, T> UpdateTables<'a, Vec<T>, ()> for PushVec<T>
    where
        T: Clone,
    {
        fn apply_first(&mut self, table: &'a mut Vec<T>) {
            table.push(self.value.clone());
        }
        fn apply_second(self, table: &mut Vec<T>) {
            table.push(self.value); // Move the value instead of cloning.
        }
    }

    struct PopVec {}
    impl PopVec {
        fn apply<'a, T>(&mut self, table: &'a mut Vec<T>) -> Option<T> {
            table.pop()
        }
    }
    impl<'a, T> UpdateTables<'a, Vec<T>, Option<T>> for PopVec {
        fn apply_first(&mut self, table: &'a mut Vec<T>) -> Option<T> {
            self.apply(table)
        }
        fn apply_second(mut self, table: &mut Vec<T>) {
            (&mut self).apply(table);
        }
    }

    /// This is an example of what not to do!
    struct MutableRef {}
    impl<'a, T> UpdateTables<'a, Vec<T>, &'a mut T> for MutableRef {
        fn apply_first(&mut self, table: &'a mut Vec<T>) -> &'a mut T {
            &mut table[0]
        }
        fn apply_second(self, table: &mut Vec<T>) {
            let _ = &mut table[0];
        }
    }

    #[test]
    fn one_write_guard() {
        // TODO: Have a multithreaded test for this.
        let writer = AsLock::<Vec<i32>>::default();
        let _wg = writer.write().unwrap();
        // let wg2 = writer.write().unwrap();
    }

    #[test]
    fn publish_update() {
        let aslock = Arc::new(AsLock::<Vec<i32>>::default());
        assert_eq!(aslock.read().unwrap().len(), 0);

        {
            let mut wg = aslock.write().unwrap();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(wg.len(), 1);
            {
                // Perform check in another thread to avoid potential deadlock
                // (calling both read and write on aslock at the same time).
                let aslock = Arc::clone(&aslock);
                thread::spawn(move || {
                    assert_eq!(aslock.read().unwrap().len(), 0);
                })
                .join()
                .unwrap();
            }
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*aslock.read().unwrap(), vec![2]);
    }

    #[test]
    fn update_tables_closure() {
        let aslock = Arc::new(AsLock::<Vec<i32>>::default());
        assert_eq!(aslock.read().unwrap().len(), 0);

        {
            let mut wg = aslock.write().unwrap();
            wg.update_tables_closure(|vec| vec.push(2));
            assert_eq!(wg.len(), 1);
            {
                // Perform check in another thread to avoid potential deadlock
                // (calling both read and write on aslock at the same time).
                let aslock = Arc::clone(&aslock);
                thread::spawn(move || {
                    assert_eq!(aslock.read().unwrap().len(), 0);
                })
                .join()
                .unwrap();
            }
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*aslock.read().unwrap(), vec![2]);
    }

    #[test]
    fn multi_apply() {
        let aslock = AsLock::<Vec<i32>>::default();
        {
            let mut wg = aslock.write().unwrap();
            wg.update_tables(PushVec { value: 2 });
            wg.update_tables(PushVec { value: 3 });
            wg.update_tables(PushVec { value: 4 });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: 5 });
        }
        assert_eq!(*aslock.read().unwrap(), vec![2, 3, 5]);
    }

    #[test]
    fn multi_publish() {
        let aslock = AsLock::<Vec<Box<i32>>>::default();
        {
            let mut wg = aslock.write().unwrap();
            wg.update_tables(PushVec { value: Box::new(2) });
            wg.update_tables(PushVec { value: Box::new(3) });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: Box::new(5) });
        }
        assert_eq!(*aslock.read().unwrap(), vec![Box::new(2), Box::new(5)]);

        {
            let mut wg = aslock.write().unwrap();
            wg.update_tables(PushVec { value: Box::new(9) });
            wg.update_tables(PushVec { value: Box::new(8) });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: Box::new(7) });
        }
        assert_eq!(
            *aslock.read().unwrap(),
            vec![Box::new(2), Box::new(5), Box::new(9), Box::new(7)]
        );

        {
            let mut wg = aslock.write().unwrap();
            wg.update_tables(PopVec {});
        }
        assert_eq!(
            *aslock.read().unwrap(),
            vec![Box::new(2), Box::new(5), Box::new(9)]
        );
    }

    #[test]
    fn multi_thread() {
        let aslock = Arc::new(AsLock::<Vec<i32>>::default());
        let aslock2 = Arc::clone(&aslock);
        let handler = thread::spawn(move || {
            while *aslock2.read().unwrap() != vec![2, 3, 5] {
                // Since commits oly happen when a WriteGuard is dropped no reader
                // will see this state.
                assert_ne!(*aslock2.read().unwrap(), vec![2, 3, 4]);
            }

            // Show multiple readers in multiple threads.
            let aslock3 = Arc::clone(&aslock2);
            let handler = thread::spawn(move || while *aslock3.read().unwrap() != vec![2, 3, 5] {});
            assert!(handler.join().is_ok());
        });

        {
            let mut wg = aslock.write().unwrap();
            wg.update_tables(PushVec { value: 2 });
            wg.update_tables(PushVec { value: 3 });
            wg.update_tables(PushVec { value: 4 });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: 5 });
        }

        assert!(handler.join().is_ok());
    }

    #[test]
    fn debug_str() {
        let aslock = AsLock::<Vec<i32>>::default();
        assert_eq!(
            format!("{:?}", aslock),
            "AsLock { num_ops_to_replay: 0, active_table: [] }"
        );
        {
            let mut wg = aslock.write().unwrap();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(
                format!("{:?}", wg),
                "WriteGuard { num_ops_to_replay: 1, standby_table: [2] }"
            );
        }
        assert_eq!(
            format!("{:?}", aslock),
            "AsLock { num_ops_to_replay: 1, active_table: [2] }"
        );
        // The aliased shared lock shows up in this debug. What we mostly care
        // about is that this says ReadGuard and shows the underlying data. It's
        // fine to update this if we ever change the underlying RwLock.
        assert_eq!(
            format!("{:?}", aslock.read().unwrap()),
            "ShardedLockReadGuard { lock: ShardedLock { data: [2] } }"
        );
    }

    #[test]
    fn panic_with_wguard_poisons_rguard() {
        let table = Arc::new(AsLock::<Vec<i32>>::default());
        let panic_handle = {
            let table = Arc::clone(&table);
            thread::spawn(move || {
                {
                    let mut wg = table.write().unwrap();
                    wg.update_tables(PushVec { value: 2 });
                }
                {
                    let mut _wg = table.write().unwrap();
                    panic!("Panic while holding the WriteGuard");
                }
            })
        };
        assert!(panic_handle.join().is_err());

        assert!(table.read().is_err());
    }

    #[test]
    #[should_panic(expected = "called `Result::unwrap()` on an `Err` value: PoisonError { .. }")]
    fn panic_with_wguard_makes_all_writes_panic() {
        let table = Arc::new(AsLock::<Vec<i32>>::default());
        let panic_handle = {
            let table = Arc::clone(&table);
            thread::spawn(move || {
                {
                    let mut wg = table.write().unwrap();
                    wg.update_tables(PushVec { value: 2 });
                }
                {
                    let mut _wg = table.write().unwrap();
                    panic!("Panic while holding the WriteGuard");
                }
            })
        };
        assert!(panic_handle.join().is_err());

        // Panics trying to unwrap `ops_to_replay`.
        let _wg = table.write();
    }

    #[test]
    fn panic_with_rguard() {
        let table = Arc::new(AsLock::<Vec<i32>>::default());
        let panic_handle = {
            let table = Arc::clone(&table);
            thread::spawn(move || {
                {
                    let mut wg = table.write().unwrap();
                    wg.update_tables(PushVec { value: 2 });
                }
                {
                    let mut _rg = table.read().unwrap();
                    panic!("Panic while holding the ReadGuard");
                }
            })
        };
        assert!(panic_handle.join().is_err());

        // RG is still valid.
        assert_eq!(*table.read().unwrap(), vec![2]);

        // WG remains valid.
        table.write().unwrap().update_tables(PushVec { value: 3 });
        assert_eq!(*table.read().unwrap(), vec![2, 3]);
    }
}
