use crate::types::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::sync;
use sync::atomic::{AtomicBool, Ordering};

/// Operations that update that data held internally must implement this
/// interface.
///
/// Users must be careful to guarantee that apply_first and apply_second cause
/// the tables to end up in the same state.
pub trait UpdateTables<T> {
    fn apply_first(&mut self, table: &mut T);

    fn apply_second(mut self: Box<Self>, table: &mut T) {
        Self::apply_first(&mut self, table)
    }
}

pub struct UpdateTablesOpsList<T, U>
where
    U: UpdateTables<T>,
{
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
    ops_to_apply: Vec<U>,

    /// The number of operations that have only been applied once. These
    /// operations have already been applied to the active table. So when we
    /// apply them to the standby_table we will consume them.
    num_ops_applied_once: usize,

    /// The compiler doesn't recognize T is used in the the requirement on U, so
    /// doesn't compile without this.
    _use_type_t: std::marker::PhantomData<T>,
}

impl<T, U> UpdateTablesOpsList<T, U>
where
    U: UpdateTables<T>,
{
    pub fn new() -> Self {
        UpdateTablesOpsList {
            ops_to_apply: vec![],
            num_ops_applied_once: 0,
            _use_type_t: std::marker::PhantomData,
        }
    }

    pub fn push(&mut self, update: U) {
        self.ops_to_apply.push(update);
    }

    pub fn append(&mut self, mut update: Vec<U>) {
        self.ops_to_apply.append(&mut update);
    }

    fn apply_updates(&mut self, table: &mut T) {
        // println!(
        //     "apply_updates {:?} {:?}",
        //     ops_to_apply.len(),
        //     num_ops_applied_once
        // );

        // Replay all ops on the standby table. This will hang until all readers
        // have returned their read guard.
        for op in self.ops_to_apply.drain(..self.num_ops_applied_once) {
            // In order to apply_second and consume the op, we need to
            // convert op into a Box.
            Box::new(op).apply_second(table);
        }
        for op in self.ops_to_apply.iter_mut() {
            op.apply_first(table);
        }

        self.num_ops_applied_once = self.ops_to_apply.len();
    }
}

impl<T: std::fmt::Debug, U> std::fmt::Debug for UpdateTablesOpsList<T, U>
where
    U: UpdateTables<T>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateTablesOpsList")
            .field("num_ops_to_apply", &self.ops_to_apply.len())
            .field("num_ops_applied_once", &self.num_ops_applied_once)
            .finish()
    }
}

pub struct Table<T> {
    /// The underlying tables. They must start out being identical, and should
    /// be eventually consistent, although realistically they will almost never
    /// be identical since ops are only performed on one at a time. At all times
    /// each table will fill a separate roll:
    /// - active_table - This is the table that is actively being read from by
    ///   readers. Whenever a reader wants to check out (non-mutably) the
    ///   underlying data, they will get this table.
    /// - standby_table - This is the table that will be updated by the writer.
    ///   When the tables are swapped it is possible that there are still
    ///   readers which hold guards to this table.
    ///
    /// Thanks to using RwLocks we are guaranteed thread safety. It is up to
    /// this wrapper to guarantee that there is the option for no contention.
    /// The first promise is that all reads, calls to active_table_guard, will
    /// never face lock contention from a writer. Writes should also have the
    /// option to publish updates if there is no lock contention, and not to
    /// hang if there is.
    table0: RwLock<T>,
    table1: RwLock<T>,

    // If true, table0 is the 'active' table, the one that readers will read
    // from on their next refresh and table1 is the 'standby' table which will
    // receive updates from a WriteGuard.
    is_table0_active: AtomicBool,
}

impl<T> Table<T> {
    pub fn active_table_guard(&self) -> RwLockReadGuard<'_, T> {
        if self.is_table0_active.load(Ordering::Relaxed) {
            self.table0.read().unwrap()
        } else {
            self.table1.read().unwrap()
        }
    }

    /// Should only be called from Writer.
    pub fn standby_table_guard(&self) -> RwLockReadGuard<'_, T> {
        if self.is_table0_active.load(Ordering::Relaxed) {
            self.table1.read().unwrap()
        } else {
            self.table0.read().unwrap()
        }
    }

    /// Perform all ops provided on the standby_table and then swap then swap
    /// the active and standby tables. This function will hang until it can gain
    /// a write lock on the standby_table.
    ///
    /// - ops_to_apply - list of updates to perform on the standby_table.
    ///   Updates that have already been performed once will be drained from the
    ///   list.
    /// - num_ops_applied_once - the number of updates in 'ops_to_apply' that
    ///   were already performed on the other table. This refers to the first N
    ///   ops. This value is updated
    ///
    ///
    /// primitives- Update batching - when updates are performed, all updates
    ///   provided are performed before the change is published to the readers.
    ///   This means that readers will never see some intermediary state where
    ///   only some of the updates have been performed.
    pub fn publish_updates<U: UpdateTables<T>>(
        &mut self,
        ops_list: &mut UpdateTablesOpsList<T, U>,
    ) {
        self.apply_updates_to_standby_table(ops_list);
        self.swap_active_and_standby();
    }
    pub fn apply_updates_to_standby_table<U: UpdateTables<T>>(
        &mut self,
        ops_list: &mut UpdateTablesOpsList<T, U>,
    ) {
        ops_list.apply_updates(&mut self.standby_table().write().unwrap());
    }

    pub fn try_to_publish_updates<U: UpdateTables<T>>(
        &mut self,
        ops_list: &mut UpdateTablesOpsList<T, U>,
    ) {
        match self.standby_table().try_write() {
            Ok(mut standby_table) => {
                ops_list.apply_updates(&mut standby_table);
            }
            _ => return,
        }
        self.swap_active_and_standby();
    }

    fn standby_table(&mut self) -> &mut RwLock<T> {
        if self.is_table0_active.load(Ordering::Relaxed) {
            &mut self.table1
        } else {
            &mut self.table0
        }
    }

    fn swap_active_and_standby(&mut self) {
        self.is_table0_active.store(
            !self.is_table0_active.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
}

impl<T> Table<T>
where
    T: Clone,
{
    pub fn new(t: T) -> Table<T> {
        Table {
            table0: RwLock::new(t.clone()),
            table1: RwLock::new(t),
            is_table0_active: AtomicBool::new(true),
        }
    }
}
