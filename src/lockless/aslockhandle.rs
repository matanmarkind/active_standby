use crate::lockless::read::{ReadGuard, Reader};
use crate::lockless::write::{WriteGuard, Writer};

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

    // Make un-sync.
    _not_sync: std::cell::UnsafeCell<fn(&T)>,
}

impl<T> AsLockHandle<T> {
    pub fn from_identical(t1: T, t2: T) -> AsLockHandle<T> {
        let writer = Writer::from_identical(t1, t2);

        // Getting a Reader at this point should be guaranteed to work since the
        // Mutex within Writer has never been locked and therefore cannot be
        // poisoned.
        let reader = writer.new_reader();

        AsLockHandle {
            writer: std::sync::Arc::new(writer),
            reader,
            _not_sync: std::cell::UnsafeCell::new(|_| {}),
        }
    }

    /// Obtain a read guard with which to inspect the active table.
    ///
    /// This is wait free since there is nothing to lock, and the Writer is
    /// responsible for never mutating the table that a ReadGuard points to.
    pub fn read(&self) -> ReadGuard<'_, T> {
        self.reader.read()
    }

    /// Create a `WriteGuard` which is used to update the underlying tables.
    ///
    /// The function is responsible for waiting for the standby table to be be
    /// free for updates & for replaying the old ops from the last WriteGuard on
    /// it.
    ///
    /// Returns `PoisonError` if the Mutex guarding the data is poisoned.
    pub fn write(&self) -> WriteGuard<'_, T> {
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

impl<T> Default for AsLockHandle<T>
where
    T: Default,
{
    fn default() -> AsLockHandle<T> {
        Self::from_identical(T::default(), T::default())
    }
}

impl<T> Clone for AsLockHandle<T> {
    fn clone(&self) -> AsLockHandle<T> {
        let writer = std::sync::Arc::clone(&self.writer);
        let reader = self.reader.clone();
        AsLockHandle {
            writer,
            reader,
            _not_sync: std::cell::UnsafeCell::new(|_| {}),
        }
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::types::*;
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
    #[should_panic(expected = "Reader is not reentrant")]
    fn reader_not_reentrant() {
        let table = AsLockHandle::<Vec<i32>>::default();
        let _rg1 = table.read();
        let _rg2 = table.read();
    }

    #[test]
    fn writer_not_reentrant() {
        let table = AsLockHandle::<Vec<i32>>::from_identical(vec![], vec![]);
        let _wg = table.write();

        // If we uncomment this line the test fails due to Mutex not being
        // re-entrant. While it is well defined that the program will not
        // proceed it is not defined how exactly the failure will occur, so we
        // cannot expect a panic as this may deadlock and hang.
        //
        // let wg2 = table.write().unwrap();
    }

    #[test]
    fn publish_update() {
        let table = AsLockHandle::<Vec<i32>>::new(vec![]);
        assert_eq!(table.read().len(), 0);

        {
            let mut wg = table.write();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(wg.len(), 1);
            assert_eq!(table.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*table.read(), vec![2]);
    }

    #[test]
    fn update_tables_closure() {
        let table = AsLockHandle::<Vec<i32>>::default();
        assert_eq!(table.read().len(), 0);

        {
            let mut wg = table.write();
            wg.update_tables_closure(|vec| vec.push(2));
            assert_eq!(wg.len(), 1);
            assert_eq!(table.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*table.read(), vec![2]);
    }

    #[test]
    fn multi_apply() {
        let table = AsLockHandle::<Vec<i32>>::default();
        {
            let mut wg = table.write();
            wg.update_tables(PushVec { value: 2 });
            wg.update_tables(PushVec { value: 3 });
            wg.update_tables(PushVec { value: 4 });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: 5 });
        }
        assert_eq!(*table.read(), vec![2, 3, 5]);
    }

    #[test]
    fn multi_publish() {
        let table = AsLockHandle::<Vec<Box<i32>>>::default();
        {
            let mut wg = table.write();
            wg.update_tables(PushVec { value: Box::new(2) });
            wg.update_tables(PushVec { value: Box::new(3) });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: Box::new(5) });
        }
        assert_eq!(*table.read(), vec![Box::new(2), Box::new(5)]);

        {
            let mut wg = table.write();
            wg.update_tables(PushVec { value: Box::new(9) });
            wg.update_tables(PushVec { value: Box::new(8) });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: Box::new(7) });
        }
        assert_eq!(
            *table.read(),
            vec![Box::new(2), Box::new(5), Box::new(9), Box::new(7)]
        );

        table.write().update_tables(PopVec {});
        assert_eq!(*table.read(), vec![Box::new(2), Box::new(5), Box::new(9)]);
    }

    #[test]
    fn multi_thread() {
        let table = AsLockHandle::<Vec<i32>>::default();
        let handler = {
            let table = table.clone();
            thread::spawn(move || {
                while *table.read() != vec![2, 3, 5] {
                    // Since commits oly happen when a WriteGuard is dropped no reader
                    // will see this state.
                    assert_ne!(*table.read(), vec![2, 3, 4]);
                }

                // Show multiple readers in multiple threads.
                let handler = {
                    let table = table.clone();
                    thread::spawn(move || while *table.read() != vec![2, 3, 5] {})
                };
                assert!(handler.join().is_ok());
            })
        };

        {
            let mut wg = table.write();
            wg.update_tables(PushVec { value: 2 });
            wg.update_tables(PushVec { value: 3 });
            wg.update_tables(PushVec { value: 4 });
            wg.update_tables(PopVec {});
            wg.update_tables(PushVec { value: 5 });
        }

        assert!(handler.join().is_ok());
    }

    #[test]
    fn writer_dropped() {
        // Show that when the Writer is dropped, Readers remain valid.
        let table;
        {
            table = AsLockHandle::<Vec<i32>>::default();

            {
                let mut wg = table.write();
                wg.update_tables(PushVec { value: 2 });
                wg.update_tables(PushVec { value: 3 });
                wg.update_tables(PushVec { value: 4 });
                wg.update_tables(PopVec {});
                wg.update_tables(PushVec { value: 5 });
            }
        }
        assert_eq!(*table.read(), vec![2, 3, 5]);
    }

    #[test]
    fn debug_str() {
        let table = AsLockHandle::<Vec<i32>>::default();
        assert_eq!(
            format!("{:?}", table),
            "AsLockHandle { writer: Writer { num_readers: 1, ops_to_replay: 0, standby_table: [] }, reader: Reader { num_readers: 1, active_table: [] } }"
        );

        {
            let mut wg = table.write();
            wg.update_tables(PushVec { value: 2 });
            assert_eq!(
                format!("{:?}", wg),
                "WriteGuard { swap_active_and_standby: true, num_readers: 1, ops_to_replay: 1, standby_table: [2] }");
        }

        // No second WriteGuard has been created, so we have yet to replay the
        // ops on the standby_table.
        assert_eq!(
            format!("{:?}", table),
            "AsLockHandle { writer: Writer { num_readers: 1, ops_to_replay: 1, standby_table: [] }, reader: Reader { num_readers: 1, active_table: [2] } }"
        );
        assert_eq!(format!("{:?}", table.read()), "[2]");
    }

    #[test]
    fn mutable_ref() {
        // Show that when the Writer is dropped, Readers remain valid.
        let table = AsLockHandle::<Vec<i32>>::default();

        {
            // Show that without giving a mutable interface we can still mutate
            // the underlying values in the table which will cause them to lose
            // consistency.
            let mut wg = table.write();
            wg.update_tables(PushVec { value: 2 });
            let mr = wg.update_tables(MutableRef {});
            *mr = 10;
        }

        assert_eq!(*table.read(), vec![10]);

        // This is bad and something clients must avoid. See comment on
        // UpdateTables trait for why this cannot be enforced by the library.
        assert_ne!(*table.read(), *table.write());
    }
}
