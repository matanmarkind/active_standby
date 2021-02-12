use crate::read::Reader;
use crate::table::Table;
use std::any::Any;
use std::error::Error;
use std::sync;
use sync::atomic::{AtomicBool, Ordering};
use sync::{Arc, RwLockWriteGuard};

type UpdateOpResult = Result<Box<dyn Any>, Box<dyn Error>>;
type UpdateOpT<T> = Box<dyn Fn(&mut T) -> UpdateOpResult>;

/// Writer is the class used to control the underlying tables. It is neither
/// Send nor Sync (although open to discussion around making it Send). If you
/// want multithreaded access to writing you must put it behind a lock of some
/// sort.
///
/// In order to interact with the underlying tables you must create a
/// WriteGuard.
///
/// Writer doesn't actually own the underlying data, so if Writer is Dropped,
/// this will not delete the tables. Instead they will only be dropped once all
/// Readers are also dropped.
pub struct Writer<T> {
    table: Arc<Table<T>>,

    // Log of operations to be performed on the active table. This gets played
    // on the active table when creating a WriteGuard as an optimization. Since
    // when a WriteGuard is dropped, we swap_active_and_standby, there will have
    // been time before creating the new WriteGuard for all the readers to exit
    // the new standby table, alleviating thread contention.
    //
    // Also holds a bool for 'is_ok' as a sanity check that if the initial
    // Result was Ok/NotOk the replay was too.
    //
    // TODO: consider holding a vec of (op, res) and comparing that each result
    // is identical on replay. This should block 1.0 since it will likely break
    // usages since now the return type will have to be PartialEq.
    ops_to_replay: Vec<(UpdateOpT<T>, bool)>,
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

/// Make sure to publish any remaning changes to the readers.
impl<T> Drop for Writer<T> {
    fn drop(&mut self) {
        // Just swapping the tables is enough. Since we are dropping the Writer,
        // we don't need to update the new standby table from 'ops_to_replay'
        // because without a writer the new standby table becomes unreachable.
        let table = unsafe {
            std::mem::transmute::<*const Table<T>, &mut Table<T>>(Arc::as_ptr(&self.table))
        };
        table.swap_active_and_standby();
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
        for (func, was_ok) in self.ops_to_replay.iter() {
            let res = func(&mut standby_table);
            assert_eq!(res.is_ok(), *was_ok);
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
/// transparent manner. When dereferencing a WriteGuard we see the state of the
/// standby_table, not the active_table which the Readers dereference. This is
/// because we cannot allow the user to directly mutate the unerlying data.
/// Instead the user has to pass in Operations, so that the Writer can make sure
/// these are performed on both the active and standby tables.
///
/// Upon Drop, a WriteGuard automatically publishes the changes to the Readers,
/// by swapping the active and standby tables. The updates are only then
/// performed on the new standby table the next time a WriteGuard is created.
/// This is to minimize thread contention. That way Readers will have a chance
/// to switch to reading from the new active table without us hanging trying to
/// WriteLock the new standby table.
pub struct WriteGuard<'w, T> {
    standby_table: RwLockWriteGuard<'w, T>,

    // Record the ops that were applied to standby_table to be replayed the next
    // time we create a WriteGuard.
    ops_to_replay: &'w mut Vec<(UpdateOpT<T>, bool)>,

    // Updated at drop.
    is_table0_active: &'w mut AtomicBool,
}

impl<'w, T> WriteGuard<'w, T> {
    /// This is where the users face the complexity of this struc, and where it
    /// most differs from a simple RwLock. Users must provide functions which
    /// update the underlying table, instead of directly touching them.
    ///
    /// It is critical the 'func' be deterministic so that it will perform the
    /// same action on both copies of the table.
    pub fn apply(&mut self, func: UpdateOpT<T>) -> UpdateOpResult {
        let res = func(&mut self.standby_table);
        self.ops_to_replay.push((func, res.is_ok()));
        res
    }
}

/// When the WriteGuard is dropped we swap the active and standby tables. We
/// don't update the new standby table until a new WriteGuard is created.
impl<'w, T> Drop for WriteGuard<'w, T> {
    fn drop(&mut self) {
        // If there were multiple writers could there be a race between loading
        // and storing? Shouldn't be germane since Writer should be Send, but
        // not Sync and there should only be 1.
        self.is_table0_active.store(
            !self.is_table0_active.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
}

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
    fn apply_update() {
        type Table = Box<Vec<i32>>;
        let mut writer = Writer::<Table>::default();
        let mut wg = writer.write();
        let res = wg.apply(Box::new(|vec: &mut Table| {
            vec.push(2);
            Ok(Box::new(()))
        }));
        assert!(res.is_ok());
    }

    #[test]
    fn publish_update() {
        type Table = Vec<Box<i32>>;
        let mut writer = Writer::<Table>::default();
        let reader = writer.new_reader();
        assert_eq!(reader.read().len(), 0);

        {
            let mut wg = writer.write();
            let res = wg.apply(Box::new(|vec: &mut Table| {
                vec.push(Box::new(2));
                Ok(Box::new(()))
            }));
            assert!(res.is_ok());
            assert_eq!(wg.len(), 1);
            assert_eq!(reader.read().len(), 0);
        }
        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*reader.read(), vec![Box::new(2)]);
    }

    #[test]
    fn multi_publish() {
        type Table = Vec<i32>;
        let mut writer = Writer::<Table>::default();

        {
            assert!(writer
                .write()
                .apply(Box::new(|vec: &mut Table| {
                    vec.push(2);
                    Ok(Box::new(()))
                }))
                .is_ok());
        }
        {
            assert!(writer
                .write()
                .apply(Box::new(|vec: &mut Table| {
                    vec.push(4);
                    Ok(Box::new(()))
                }))
                .is_ok());
        }
        {
            assert!(writer
                .write()
                .apply(Box::new(|vec: &mut Table| {
                    vec.push(6);
                    Ok(Box::new(()))
                }))
                .is_ok());
        }
        {
            assert!(writer
                .write()
                .apply(Box::new(|vec: &mut Table| {
                    vec.push(8);
                    Ok(Box::new(()))
                }))
                .is_ok());
        }

        let reader = writer.new_reader();
        assert_eq!(*reader.read(), vec![2, 4, 6, 8]);
    }

    #[test]
    fn multi_apply() {
        let mut writer = Writer::<Vec<i32>>::default();
        {
            let mut wg = writer.write();
            assert!(wg
                .apply(Box::new(|vec: &mut Vec<i32>| {
                    vec.push(2);
                    Ok(Box::new(()))
                }))
                .is_ok());
            assert!(wg
                .apply(Box::new(|vec: &mut Vec<i32>| {
                    vec.push(4);
                    Ok(Box::new(()))
                }))
                .is_ok());
            assert!(wg
                .apply(Box::new(|vec: &mut Vec<i32>| {
                    vec.pop();
                    Ok(Box::new(()))
                }))
                .is_ok());
        }
        let reader = writer.new_reader();
        assert_eq!(*reader.read(), vec![2]);
    }

    #[test]
    fn multi_thread() {
        let mut writer = Writer::<Vec<i32>>::default();
        let reader = writer.new_reader();
        let handler = thread::spawn(move || while *reader.read() != vec![1, 2, 3] {});

        {
            let mut wg = writer.write();
            assert!(wg
                .apply(Box::new(|vec: &mut Vec<i32>| {
                    vec.push(1);
                    vec.push(2);
                    vec.push(3);
                    Ok(Box::new(()))
                }))
                .is_ok());
        }

        assert!(handler.join().is_ok());
    }
    // multi_reader
}
