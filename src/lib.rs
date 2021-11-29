//! A concurrency library for high concurrency reads.
//!
//! This library is named after the 2 (identical) tables that we hold
//! internally:
//! - Active - this is the table that all Readers view. This table will never be
//!   write locked, so readers never face contention.
//! - Standby - this is the table the the Writer mutates. A writer should face
//!   minimal contention retrieving this table for mutation since Readers move
//!   to the Active table when the tables are swapped.
//!
//! The cost of providing no contention to readers, and minimal contention to
//! writers is:
//! 1. Memory - Internally we hold 2 copies of the underlying type the user
//!    created. This is needed to allow there to always be a table that Readers
//!    can check out without contention.
//! 2. Writer thread CPU usage - The writer must apply all updates twice, once
//!    to each table. Lock contention for the writer should be less than with a
//!    plain RwLock due to Readers using the active_table.
//!
//! The usage is meant to be similar to a RwLock. Some of the inspiration came
//! from the [left_right](https://crates.io/crates/left-right) crate, so feel
//! free to check that out. The main differences focus on trying to simplify the
//! client (creating data structures) and user (using data structures)
//! experiences; primarily focused on trying to mimic the API/usage of an
//! RwLock.
//!
//! There are 2 flavors of this algorithm that we offer:
//! 1. Lockless - this variant trades off increased performance against changing
//!    the API to be less like an RwLock. This avoids the cost of performing
//!    synchronization on reads, but this requires that each thread/task that is
//!    going to access the tables, registers in advance. Therefore this centers
//!    around the AsLockHandle, which is conceptually similar to Arc\<RwLock>
//!    (i.e. you clone the AsLockHandle and pass the new one to other threads).
//! 2. Shared - this centers around using an AsLock, which is meant to feel like
//!    an RwLock. These structs can be shared between threads by cloning &
//!    sending an Arc\<AsLock> (like with RwLock). The main difference is that
//!    instead of using AsLock\<Vec\<T>>, you would use vec::shared::AsLock\<T>.
//!    This is because both tables must be updated, so users can't just
//!    dereference and mutate the underlying table.
//!
//! An example of where the shared variant can be preferable is a Tonic service.
//! There you don't spawn a set of tasks/threads where you can pass each of them
//! an AsLockHandle. You can use an AsLock though and receive a similar
//! experience.
//!
//! A result of having the two separate tables is that batching becomes more
//! viable. You can grab a ReadGuard, and handle multiple requests without
//! worrying about starving the writer since the writer can work on the standby
//! table, as opposed to with an RwLock. This means multiple requests can be
//! handled without having to relock the active_table. Similarly you can batch
//! with the Writer without starving the Readers.
//!
//! We provide 2 modules:
//! 1. primitives - The components used to build data structures in the
//!    active_standby model. Users usually don't need to utilize the primitives
//!    and can instead either utilize the pre-made collections, or generate the
//!    wrapper for their struct using one of the macros and then just implement
//!    the mutable API for the generated WriteGuard.
//! 2. collections - Shared and lockless active_standby structs for common
//!    collections. Each table type has its own AsLock (shared) / AsLockHandle
//!    (lockless), as opposed to RwLock where you simply pass in the table. This
//!    is because users can't simply gain write access to the underlying table
//!    and then mutate it. Instead mutations are done through UpdateTables so
//!    that both tables will be updated.
//!
//! Example:
//! ```
//! use std::thread::sleep;
//! use std::time::Duration;
//! use std::sync::Arc;
//! use active_standby::primitives::UpdateTables;
//!
//! // Client's must implement the mutable interface that they want to offer users
//! // of their active standby data structure. This is not automatically generated.
//! struct AddOne {}
//! impl<'a> UpdateTables<'a, i32, ()> for AddOne {
//!     fn apply_first(&mut self, table: &'a mut i32) {
//!         *table = *table + 1;
//!     }
//!     fn apply_second(mut self, table: &mut i32) {
//!         self.apply_first(table);
//!     }
//! }
//!
//! pub mod lockless {
//!     active_standby::generate_lockless_aslockhandle!(i32);
//!
//!     impl<'w> WriteGuard<'w> {
//!         pub fn add_one(&mut self) {
//!             self.guard.update_tables(super::AddOne {})
//!         }
//!     }
//! }
//!
//! pub mod shared {
//!     active_standby::generate_shared_aslock!(i32);
//!
//!     impl<'w> WriteGuard<'w> {
//!         pub fn add_one(&mut self) {
//!             self.guard.update_tables(super::AddOne {})
//!         }
//!     }
//! }
//!
//! fn run_lockless() {
//!     let table = lockless::AsLockHandle::new(0);
//!     let table2 = table.clone();
//!     let handle = std::thread::spawn(move || {
//!         while *table2.read() != 1 {
//!             sleep(Duration::from_micros(100));
//!         }
//!     });
//!
//!     {
//!         let mut wg = table.write();
//!         wg.add_one();
//!     }
//!     handle.join();
//! }
//!
//! fn run_shared() {
//!     let table = Arc::new(shared::AsLock::new(0));
//!     let table2 = Arc::clone(&table);
//!     let handle = std::thread::spawn(move || {
//!         while *table2.read() != 1 {
//!             sleep(Duration::from_micros(100));
//!         }
//!     });
//!
//!     {
//!         let mut wg = table.write();
//!         wg.add_one();
//!     }
//!     handle.join();
//! }
//!
//! fn main() {
//!     run_lockless();
//!     run_shared();
//! }
//! ```
//!
//! If your table has large elements, you may want to save memory by only
//! holding each element once (e.g. vec::AsLockHandle<Arc<i32>>). This can be
//! done safely so long as no elements of the table are mutated, only inserted
//! and removed. Using a vector as an example, if you wanted a function that
//! increases the value of the first element by 1, you would not increment the
//! value behind the Arc. You would reassign the first element to a new Arc with
//! the incremented value.

mod macros;
pub(crate) mod types;

mod lockless;
mod shared;

/// The components used to build data structures in the active_standby model.
/// Users should usually don't need to utilize the primitives and can instead
/// either utilize the pre-made collections, or generate the wrapper for their
/// struct using one of the macros and then just implement the mutations for the
/// generated WriteGuard.
pub mod primitives {
    pub use crate::types::{RwLock, RwLockReadGuard, RwLockWriteGuard, UpdateTables};
    pub mod lockless {
        pub use crate::lockless::aslockhandle::AsLockHandle;
        pub use crate::lockless::read::{ReadGuard, Reader};
        pub use crate::lockless::write::{WriteGuard, Writer};
    }
    pub mod shared {
        pub use crate::shared::aslock::{AsLock, WriteGuard};
    }
}

/// Shared and lockless active_standby structs for common collections. Each
/// table type has its own AsLock (shared) / AsLockHandle (lockless), as opposed
/// to RwLock where you simply pass in the table. This is because users can't
/// simply gain write access to the underlying table and then mutate it. Instead
/// mutations are done through UpdateTables so that both tables will be updated.
pub mod collections;