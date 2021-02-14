use crate::read::Reader;
use crate::table::Table;
use crate::types::RwLockWriteGuard;
use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// This is the trait for functions that update the underlying tables. This is
/// the most risky part that users will have to take care with. Specifically to
/// make sure that both apply_first and apply_second perform identical changes
/// on the 2 tables.
pub trait UpdateTables<T> {
    fn apply_first(&mut self, table: &mut T) -> Box<dyn Any>;

    fn apply_second(mut self: Box<Self>, table: &mut T) -> Box<dyn Any> {
        Self::apply_first(&mut self, table)
    }
}

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
pub struct Writer<T> {
    table: Arc<Table<T>>,

    /// Log of operations to be performed on the active table. This gets played
    /// on the standby table when creating a WriteGuard as an optimization.
    /// Since when a WriteGuard is dropped, we swap the active and standby
    /// tables, by waiting until the next time a WriteGuard is created we give
    /// the readers time to switch to reading from the new active_table. This
    /// hopefully reduces contention when the writer tries to lock the new
    /// standby_table.
    ///
    /// We could make the Writer Send + Sync if we instead gave up on this
    /// optimization and moved ops_to_replay into WriteGuard, and had WriteGuard
    /// perform these ops on Drop. I think this optimization is worth the need
    /// for the user to wrap Writer in a Mutex though.
    ops_to_replay: Vec<Box<dyn UpdateTables<T>>>,
}

impl<T> Writer<T>
where
    T: Clone,
{
    pub fn new_from_empty(t: T) -> Writer<T> {
        Writer {
            table: Arc::new(Table::new_from_empty(t)),
            ops_to_replay: vec![],
        }
    }
}

impl<T> Writer<T>
where
    T: Default + Clone,
{
    pub fn default() -> Writer<T> {
        Self::new_from_empty(T::default())
    }
}

impl<T> Writer<T> {
    /// Create a WriteGuard to allow users to update the the data. There will
    /// only be 1 WriteGuard at a time.
    ///
    /// This function may be slow because:
    /// 1. Lock contention on the standby_table. This can occur if a ReadGuard
    ///    which was created before the last WriteGuard was dropped, still has
    ///    not itself been dropped.
    /// 2. Replaying all of the updates that were applied to the last
    ///    WriteGuard.
    pub fn write(&mut self) -> WriteGuard<'_, T> {
        // We rely on knowing that this is the only Writer and it can only call
        // to 'write' when there are no existing WriteGuards.
        let table = unsafe {
            std::mem::transmute::<*const Table<T>, &mut Table<T>>(Arc::as_ptr(&self.table))
        };

        // Replay all ops on the standby table. This will hang until all readers
        // have returned their read guard.
        let (mut standby_table, is_table0_active) = table.write_guard();
        for op in self.ops_to_replay.drain(..) {
            op.apply_second(&mut standby_table);
        }
        self.ops_to_replay.clear();

        WriteGuard {
            standby_table,
            ops_to_replay: &mut self.ops_to_replay,
            is_table0_active,
        }
    }

    pub fn new_reader(&self) -> Reader<T> {
        Reader::new(Arc::clone(&self.table))
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
pub struct WriteGuard<'w, T> {
    standby_table: RwLockWriteGuard<'w, T>,

    // Record the ops that were applied to standby_table to be replayed the next
    // time we create a WriteGuard.
    ops_to_replay: &'w mut Vec<Box<dyn UpdateTables<T>>>,

    // Updated at drop.
    is_table0_active: &'w mut AtomicBool,
}

impl<'w, T> WriteGuard<'w, T> {
    /// This is where the users face the complexity of this struct, and where it
    /// most differs from a simple RwLock. Users must provide functions which
    /// update the underlying table, instead of directly touching them.
    ///
    /// It is critical the 'func' be deterministic so that it will perform the
    /// same action on both copies of the table.
    ///
    /// op implicitly requires that it be 'static, since it this is implicit in
    /// traits. This makes sense since we pass in ownership of the op to Writer
    /// and can't keep it tied to an outside object.
    /// 
    /// TODO: See if there is a way to not require 'op' be 'static.
    pub fn update_tables(&mut self, mut op: Box<dyn UpdateTables<T>>) -> Box<dyn Any> {
        let res = op.apply_first(&mut self.standby_table);
        self.ops_to_replay.push(op);
        res
    }
}

/// When the WriteGuard is dropped we swap the active and standby tables. We
/// don't update the new standby table until a new WriteGuard is created.
impl<'w, T> Drop for WriteGuard<'w, T> {
    fn drop(&mut self) {
        // Make sure to drop the write guard first to guarantee that readers
        // never face contention.
        drop(&mut self.standby_table);

        // Swap the active and standby tables.
        self.is_table0_active.store(
            !self.is_table0_active.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
}

/// Dereferencing the WriteGuard will let you see the state of the
/// standby_table. If you want to inspect the state of the active_table you must
/// go through a Reader.
impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
    type Target = RwLockWriteGuard<'w, T>;
    fn deref(&self) -> &Self::Target {
        &self.standby_table
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::thread;

    struct PushVec<T> {
        value: T,
    }
    impl<T> UpdateTables<Vec<T>> for PushVec<T>
    where
        T: Clone,
    {
        fn apply_first(&mut self, table: &mut Vec<T>) -> Box<dyn Any> {
            table.push(self.value.clone());
            Box::new(())
        }
        fn apply_second(self: Box<Self>, table: &mut Vec<T>) -> Box<dyn Any> {
            table.push(self.value); // Move the value instead of cloning.
            Box::new(())
        }
    }

    struct PopVec {}
    impl<T> UpdateTables<Vec<T>> for PopVec {
        fn apply_first(&mut self, table: &mut Vec<T>) -> Box<dyn Any> {
            table.pop();
            Box::new(())
        }
    }

    #[test]
    fn one_guard() {
        let mut writer = Writer::<Vec<i32>>::default();
        let _wg = writer.write();

        // If we uncomment this line the program fails to compile due to a
        // second mutable borrow. This is what we want to guarantee there can
        // only be 1 WriteGuard at a time.
        //
        // let wg2 = writer.write();
    }

    #[test]
    fn publish_update() {
        let mut writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        assert_eq!(reader.read().len(), 0);

        {
            let mut wg = writer.write();
            let res = wg.update_tables(Box::new(PushVec { value: 2 }));
            assert!(res.is::<()>());
            assert_eq!(wg.len(), 1);
            assert_eq!(reader.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*reader.read(), vec![2]);
    }

    #[test]
    fn multi_apply() {
        let mut writer = Writer::<Vec<i32>>::default();
        {
            let mut wg = writer.write();
            wg.update_tables(Box::new(PushVec { value: 2 }));
            wg.update_tables(Box::new(PushVec { value: 3 }));
            wg.update_tables(Box::new(PushVec { value: 4 }));
            wg.update_tables(Box::new(PopVec {}));
            wg.update_tables(Box::new(PushVec { value: 5 }));
        }
        let reader = writer.new_reader();
        assert_eq!(*reader.read(), vec![2, 3, 5]);
    }

    #[test]
    fn multi_publish() {
        let mut writer = Writer::<Vec<Box<i32>>>::default();
        {
            let mut wg = writer.write();
            wg.update_tables(Box::new(PushVec { value: Box::new(2) }));
            wg.update_tables(Box::new(PushVec { value: Box::new(3) }));
            wg.update_tables(Box::new(PopVec {}));
            wg.update_tables(Box::new(PushVec { value: Box::new(5) }));
        }
        let reader = writer.new_reader();
        assert_eq!(*reader.read(), vec![Box::new(2), Box::new(5)]);

        {
            let mut wg = writer.write();
            wg.update_tables(Box::new(PushVec { value: Box::new(9) }));
            wg.update_tables(Box::new(PushVec { value: Box::new(8) }));
            wg.update_tables(Box::new(PopVec {}));
            wg.update_tables(Box::new(PushVec { value: Box::new(7) }));
        }
        let reader = writer.new_reader();
        assert_eq!(
            *reader.read(),
            vec![Box::new(2), Box::new(5), Box::new(9), Box::new(7)]
        );

        {
            let mut wg = writer.write();
            wg.update_tables(Box::new(PopVec {}));
        }
        let reader = writer.new_reader();
        assert_eq!(*reader.read(), vec![Box::new(2), Box::new(5), Box::new(9)]);
    }

    #[test]
    fn multi_thread() {
        let mut writer = Writer::<Vec<i32>>::default();
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
            wg.update_tables(Box::new(PushVec { value: 2 }));
            wg.update_tables(Box::new(PushVec { value: 3 }));
            wg.update_tables(Box::new(PushVec { value: 4 }));
            wg.update_tables(Box::new(PopVec {}));
            wg.update_tables(Box::new(PushVec { value: 5 }));
        }

        assert!(handler.join().is_ok());
    }

    #[test]
    fn writer_dropped() {
        // Show that when the Writer is dropped, Readers remain valid.
        let reader;
        {
            let mut writer = Writer::<Vec<i32>>::default();
            reader = writer.new_reader();

            {
                let mut wg = writer.write();
                wg.update_tables(Box::new(PushVec { value: 2 }));
                wg.update_tables(Box::new(PushVec { value: 3 }));
                wg.update_tables(Box::new(PushVec { value: 4 }));
                wg.update_tables(Box::new(PopVec {}));
                wg.update_tables(Box::new(PushVec { value: 5 }));
            }
        }
        assert_eq!(*reader.read(), vec![2, 3, 5]);
    }
}
