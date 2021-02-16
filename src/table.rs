use crate::types::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::sync;
use sync::atomic::{AtomicBool, Ordering};

/// This is the trait for functions that update the underlying tables. This is
/// the most risky part that users will have to take care with. Specifically to
/// make sure that both apply_first and apply_second perform identical changes
/// on the 2 tables.
pub trait UpdateTables<T> {
    fn apply_first(&mut self, table: &mut T);

    fn apply_second(mut self: Box<Self>, table: &mut T) {
        Self::apply_first(&mut self, table)
    }
}

pub struct Table<T> {
    // Possible alternative design is to stop having the bool, and instead have
    // just the 2 tables as AtomicPtr, and WriteGuard will swap the pointers.
    // The tradeoff I am most interested in is the performance of read_guard.
    // This is a tradeoff between branching and indirection (aka CPU speed v.
    // Memory loading speed). I lean now towards branching since CPUs seem to be
    // quite fast and other examples indicate to me that this could be better
    // (e.g. C++ std::string switched from just following a pointer to storing
    // short strings locally https://www.youtube.com/watch?v=kPR8h4-qZdk)
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

    pub fn publish_updates<U: UpdateTables<T>>(
        &mut self,
        ops_to_apply: &mut Vec<U>,
        num_ops_applied_once: &mut usize,
    ) {
        Self::update_standby_table(
            self.standby_table().write().unwrap(),
            ops_to_apply,
            num_ops_applied_once,
        );
        self.swap_active_and_standby();
    }

    pub fn try_to_publish_updates<U: UpdateTables<T>>(
        &mut self,
        ops_to_apply: &mut Vec<U>,
        num_ops_applied_once: &mut usize,
    ) {
        match self.standby_table().try_write() {
            Ok(standby_table) => {
                Self::update_standby_table(standby_table, ops_to_apply, num_ops_applied_once);
            }
            _ => return,
        }
        self.swap_active_and_standby();
    }

    /// Private functions used for updating the tables.
    fn update_standby_table<U: UpdateTables<T>>(
        mut standby_table: RwLockWriteGuard<'_, T>,
        ops_to_apply: &mut Vec<U>,
        num_ops_applied_once: &mut usize,
    ) {
        // println!(
        //     "update_standby_table {:?} {:?}",
        //     ops_to_apply.len(),
        //     num_ops_applied_once
        // );

        // Replay all ops on the standby table. This will hang until all readers
        // have returned their read guard.
        for op in ops_to_apply.drain(..*num_ops_applied_once) {
            // In order to apply_second and consume the op, we need to
            // convert op into a Box.
            Box::new(op).apply_second(&mut standby_table);
        }
        for op in ops_to_apply.iter_mut() {
            op.apply_first(&mut standby_table);
        }

        *num_ops_applied_once = ops_to_apply.len();
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
