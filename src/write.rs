// A writer handle to a let-right data structure. Unlike ReadHandle there is no
// guard here.
use crate::table::Table;
use std::sync;
use sync::atomic::{AtomicBool, Ordering};
use sync::{Arc, RwLockWriteGuard};

// WARNING: The main risk for the user is making sure that each operation is
// deterministic across both tables.
type UpdateOpResult = Result<Box<dyn std::any::Any>, Box<dyn std::error::Error>>;
type UpdateOpT<T> = Box<dyn Fn(&mut T) -> UpdateOpResult>;

// TODO: Guarantee Writer is not Sync, but it is Send...
pub struct Writer<T> {
    table: Arc<Table<T>>,

    // Log of operations to be performed on the active table. This gets played
    // on the active table when creating a WriteGuard as an optimization. Since
    // when a WriteGuard is dropped, we swap_active_and_standby, there will have
    // been time before creating the new WriteGuard for all the readers to exit
    // the new standby table, alleviating thread contention.
    //
    // Also holds a bool for 'is_ok' as a sanity check that if the initial
    // Result was OK the replay was too, or they were both not OK.
    //
    // TODO: consider holding a vec of (op, res) and comparing that each result
    // is identical on replay.
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

// Taking a mutable reference to the contents of Writer will guarantee we only
// spawn 1 WriteGuard at a time from a given Writer.
pub struct WriteGuard<'w, T> {
    standby_table: RwLockWriteGuard<'w, T>,

    // Record the ops that were applied to standby_table to be replayed the next
    // time we create a WriteGuard.
    ops_to_replay: &'w mut Vec<(UpdateOpT<T>, bool)>,

    // Updated at drop.
    is_table0_active: &'w mut AtomicBool,
}

impl<'w, T> WriteGuard<'w, T> {
    pub fn new(writer: &'w mut Writer<T>) -> WriteGuard<'w, T> {
        // We rely on knowing that this is the only Writer/WriteGuard.
        let table = unsafe {
            std::mem::transmute::<*const Table<T>, &mut Table<T>>(Arc::as_ptr(&writer.table))
        };

        // Replay all ops on the standby table. This will hang until all readers
        // have returned their read guard.
        let (mut standby_table, is_table0_active) = table.write_guard();
        for (func, was_ok) in writer.ops_to_replay.iter() {
            let res = func(&mut standby_table);
            assert_eq!(res.is_ok(), *was_ok);
        }
        writer.ops_to_replay.clear();

        WriteGuard {
            standby_table,
            ops_to_replay: &mut writer.ops_to_replay,
            is_table0_active,
        }
    }

    pub fn apply(&mut self, func: UpdateOpT<T>) -> UpdateOpResult {
        let res = func(&mut self.standby_table);
        self.ops_to_replay.push((func, res.is_ok()));
        res
    }
}

impl<'w, T> Drop for WriteGuard<'w, T> {
    fn drop(&mut self) {
        // If there were multiple writers could there be a race between loading
        // and storing? Shouldn't be germane since Writer should be Send, but
        // not Sync and there should only be 1.
        self.is_table0_active.store(
            !self.is_table0_active.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        // Note that we don't update the new standby_table now, but only upon
        // creation of the next WriteGuard. This is because in order to update
        // the standby_tale, we need all readers to stop holding a guard to it.
        // So in order to minimize time spent waiting for the lock, we wait to
        // do that until the next WriteGuard is created.
    }
}
