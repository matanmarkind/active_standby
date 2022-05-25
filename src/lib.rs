//! A library for high concurrency reads.
//!
//! This library is named after the 2 (identical) tables that are held internally:
//! - Active - this is the table that all Readers view. This table will never be
//!   write locked, so readers never face contention.
//! - Standby - this is the table that the writers mutate. A writer should face
//!   minimal contention retrieving this table since Readers move to the Active
//!   table whenever calling `.read()`.
//!
//! There are 2 ways to use this crate:
//! 1. Direct interaction with AsLock. This is more flexible since users can pass
//!    in any struct they want and mutate it however they choose. All updates
//!    though, will need to be done by passing a function instead of via mutable
//!    methods (`UpdateTables` trait).
//! 2. Using collections which are built out of the primitives but which provide an
//!    API similar to RwLock<T>; writers can directly call to methods without
//!    having to provide a mutator function.
//!
//! There are 2 flavors/modules:
//! 1. Lockless - this variant trades off increased performance against changing the
//!    API to be less like a `RwLock`. This centers around the `AsLockHandle`, which
//!    is conceptually similar to `Arc<RwLock>` (meaning a separate `AsLockHandle`
//!    per thread/task).
//! 2. Sync - this centers around using an `AsLock`, which is meant to feel like a
//!    `RwLock`. The main difference is that you still cannot gain direct write
//!    access to the underlying table due to the need to keep them identical.
//!
//! The cost of minimizing contention is:
//! 1. Memory - Internally there are 2 copies of the underlying type the user
//!    created. This is needed to allow there to always be a table that Readers can
//!    access out without contention.
//! 2. CPU - The writer must apply all updates twice, once to each table. Lock
//!    contention for the writer should be less than with a plain RwLock due to
//!    Readers using the active_table, so it's possible that write times themselves
//!    will drop.
//!
//! ### Example
//! This example builds a toy collection, wrapping the active_standby struct
//! in a familiar RwLock interface. For more examples check out the source code
//! of the exported collections.
//! ```rust
//! use std::thread::sleep;
//! use std::time::Duration;
//! use std::sync::Arc;
//! use active_standby::UpdateTables;
//!
//! // Client's should implement the mutable interface that they want to offer users
//! // of their active standby data structure. This is not automatically generated.
//! struct AddOne {}
//!
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
//!     impl<'w> AsLockWriteGuard<'w> {
//!         pub fn add_one(&mut self) {
//!             self.guard.update_tables(super::AddOne {})
//!         }
//!     }
//! }
//!
//! pub mod sync {
//!     active_standby::generate_sync_aslock!(i32);
//!
//!     impl<'w> AsLockWriteGuard<'w> {
//!         pub fn add_one(&mut self) {
//!             self.guard.update_tables(super::AddOne {})
//!         }
//!     }
//! }
//!
//! fn run_lockless() {
//!     let table = lockless::AsLockHandle::new(0);
//!     let table2 = table.clone();
//!
//!     let handle = std::thread::spawn(move || {
//!         while *table2.read() != 1 {
//!             sleep(Duration::from_micros(100));
//!         }
//!     });
//!
//!     table.write().add_one();
//!     handle.join();
//! }
//!
//! fn run_sync() {
//!     let table = Arc::new(sync::AsLock::new(0));
//!     let table2 = Arc::clone(&table);
//!
//!     let handle = std::thread::spawn(move || {
//!         while *table2.read() != 1 {
//!             sleep(Duration::from_micros(100));
//!         }
//!     });
//!
//!     table.write().add_one();
//!     handle.join();
//! }
//!
//! fn main() {
//!     run_lockless();
//!     run_sync();
//! }
//! ```
//!
//! ## Testing
//! There are a number of tests that come with active_standby (see
//! tests/tests_script.sh for examples):
//!
//! [unittests](https://doc.rust-lang.org/book/ch11-01-writing-tests.html)
//!
//! [benchmarks](https://doc.rust-lang.org/unstable-book/library-features/test.html)
//!
//! [loom](https://crates.io/crates/loom)
//!
//! [LLVM Sanitizers](https://doc.rust-lang.org/beta/unstable-book/compiler-flags/sanitizer.html)
//!
//! [Miri](https://github.com/rust-lang/miri)
//!
//! [Rudra](https://github.com/sslab-gatech/Rudra)

mod macros;
pub(crate) mod types;

mod collections;
mod primitives;

pub use crate::types::UpdateTables;
pub mod lockless {

    /// Premade structs which wrap standard collection in the active standby
    /// model. This allows for the API to match RwLock<T> while under the hood
    /// the active standby model works its magic. AsLockReadGuard is generic
    /// since only writes require a special API for active_standby.
    ///
    /// To see the mutable interface of a given collection, check out the
    /// `*WriteGuard` exposed here. Following the linked `AsLockWriteGuard`
    /// will bring you to the generic one for all structs.
    pub mod collections {
        // Inline the re-export to make rustdocs more readable.
        #[doc(inline)]
        pub use crate::collections::btreemap::lockless::{
            AsLockHandle as AsBTreeMapHandle, AsLockWriteGuard as AsBTreeMapWriteGuard,
        };
        #[doc(inline)]
        pub use crate::collections::btreeset::lockless::{
            AsLockHandle as AsBTreeSetHandle, AsLockWriteGuard as AsBTreeSetWriteGuard,
        };
        #[doc(inline)]
        pub use crate::collections::hashmap::lockless::{
            AsLockHandle as AsHashMapHandle, AsLockWriteGuard as AsHashMapWriteGuard,
        };
        #[doc(inline)]
        pub use crate::collections::hashset::lockless::{
            AsLockHandle as AsHashSetHandle, AsLockWriteGuard as AsHashSetWriteGuard,
        };
        #[doc(inline)]
        pub use crate::collections::vec::lockless::{
            AsLockHandle as AsVecHandle, AsLockWriteGuard as AsVecWriteGuard,
        };
    }
    pub use crate::primitives::lockless::{AsLockHandle, AsLockReadGuard, AsLockWriteGuard};
}

pub mod sync {
    /// Premade structs which wrap standard collection in the active standby
    /// model. This allows for the API to match RwLock<T> while under the hood
    /// the active standby model works its magic. AsLockReadGuard is generic
    /// since only writes require a special API for active_standby.
    ///
    /// To see the mutable interface of a given collection, check out the
    /// `*WriteGuard` exposed here. Following the linked `AsLockWriteGuard`
    /// will bring you to the generic one for all structs.
    pub mod collections {
        // Inline the re-export to make rustdocs more readable.
        #[doc(inline)]
        pub use crate::collections::btreemap::sync::{
            AsLock as AsBTreeMap, AsLockWriteGuard as AsBTreeMapWriteGuard,
        };
        #[doc(inline)]
        pub use crate::collections::btreeset::sync::{
            AsLock as AsBTreeSet, AsLockWriteGuard as AsBTreeSetWriteGuard,
        };
        #[doc(inline)]
        pub use crate::collections::hashmap::sync::{
            AsLock as AsHashMap, AsLockWriteGuard as AsHashMapWriteGuard,
        };
        #[doc(inline)]
        pub use crate::collections::hashset::sync::{
            AsLock as AsHashSet, AsLockWriteGuard as AsHashSetWriteGuard,
        };
        #[doc(inline)]
        pub use crate::collections::vec::sync::{
            AsLock as AsVec, AsLockWriteGuard as AsVecWriteGuard,
        };
    }
    pub use crate::primitives::sync::{AsLock, AsLockReadGuard, AsLockWriteGuard};
}
