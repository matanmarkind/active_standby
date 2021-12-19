// Loom & crossbeam return the std PoisonError, so no need to use conditional
// compilation.
pub use std::sync::PoisonError;
pub type LockResult<Guard> = Result<Guard, PoisonError<Guard>>;

#[cfg(loom)]
pub(crate) use loom::hint::spin_loop;
#[cfg(loom)]
pub(crate) use loom::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
#[cfg(loom)]
pub(crate) use loom::sync::{Arc, Mutex, MutexGuard};
#[cfg(loom)]
pub type RwLock<T> = loom::sync::RwLock<T>;
#[cfg(loom)]
pub type RwLockReadGuard<'r, T> = loom::sync::RwLockReadGuard<'r, T>;
#[cfg(loom)]
pub type RwLockWriteGuard<'w, T> = loom::sync::RwLockWriteGuard<'w, T>;
#[cfg(loom)]
pub(crate) fn fence(ord: Ordering) {
    if let Ordering::Acquire = ord {
    } else {
        // FIXME: loom only supports acquire fences at the moment.
        // https://github.com/tokio-rs/loom/issues/117
        // let's at least not panic...
        // this may generate some false positives (`SeqCst` is stronger than `Acquire`
        // for example), and some false negatives (`Relaxed` is weaker than `Acquire`),
        // but it's the best we can do for the time being.
    }
    loom::sync::atomic::fence(Ordering::Acquire)
}

#[cfg(not(loom))]
pub(crate) use std::hint::spin_loop;
#[cfg(not(loom))]
pub(crate) use std::sync::atomic::{fence, AtomicPtr, AtomicUsize, Ordering};
#[cfg(not(loom))]
pub(crate) use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(not(loom))]
pub type RwLock<T> = crossbeam::sync::ShardedLock<T>;
#[cfg(not(loom))]
pub type RwLockReadGuard<'r, T> = crossbeam::sync::ShardedLockReadGuard<'r, T>;
#[cfg(not(loom))]
pub type RwLockWriteGuard<'w, T> = crossbeam::sync::ShardedLockWriteGuard<'w, T>;

/// Operations that update the data held internally. Users mutate the tables by
/// implementing this trait for each function to be performed on the tables. For
/// examples check the implementations for the collections.
///
/// Users must be careful to guarantee that apply_first and apply_second cause
/// the tables to end up in the same state. They also must be certain not to use
/// the return value to mutate the underlying table, since this likely can't be
/// mimiced in 'apply_second', which will lead to divergent tables.
///
/// It is *highly* discouraged to create updates which return mutable references
/// to the table's internals. E.g:
///
///```
/// # use active_standby::primitives::UpdateTables;
/// struct MutableRef {}
///
/// impl<'a, T> UpdateTables<'a, Vec<T>, &'a mut T> for MutableRef {
///    fn apply_first(&mut self, table: &'a mut Vec<T>) -> &'a mut T {
///        &mut table[0]
///    }
///    fn apply_second(self, table: &mut Vec<T>) {
///        &mut table[0];
///    }
/// }
/// ```
///
/// Even without the explicit lifetime, which allows for mutable references,
/// this issue is still possible.
///
/// ```
/// use std::sync::Arc;
/// use std::cell::RefCell;
///
/// fn ret_owned_value<T: Clone>(opt : &Vec<T>) -> T {
///     opt[0].clone()
/// }
///
/// fn main() {
///     let opt = vec![Arc::new(RefCell::new(3)), Arc::new(RefCell::new(5))];
///     let opt_ref = ret_owned_value(&opt);
///     *opt_ref.borrow_mut() += 1;
///     println!("{:?}, {:?}", opt_ref, opt);
///     // prints: "RefCell { value: 4 }, [RefCell { value: 4 }, RefCell { value: 5 }]"
/// }
/// ```
///
/// Therefore it is also highly recommended not to include types that allow for
/// interior mutability, since that can lead to the caller returning a reference
/// to part of an underlying table. If the caller then mutates this outside of
/// UpdateTables, this is can cause divergence between the tables since
/// apply_second isn't aware of this mutation.
///
/// If the table holds large elements, a user may want to save memory by having
/// Table<Arc\<T>>. This can be done safely so long as UpdateTables never
/// mutates the value pointed to (T). UpdateTables may instead only update the Table
/// by inserting and removing elements.
pub trait UpdateTables<'a, T, R> {
    fn apply_first(&mut self, table: &'a mut T) -> R;

    /// Unfortunately we can't offer a default implementationt to call
    /// 'apply_first'. This is because we can't constrain 'apply_second' with a
    /// lifetime on 'table' because this would mean that each update has a
    /// unique type in 'apply_second', making it impossible for us to hold
    /// 'ops_to_replay' since each op would have a different type.
    fn apply_second(self, table: &mut T);
}
