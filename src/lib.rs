//! A concurrency library for high concurrency reads with a single writer.
//! 
//! This library is named after the 2 (identical) tables that we hold internally:
//! - Active - this is the table that all Readers view. This table will never be
//!   write locked, so readers never face contention.
//! - Standby - this is the table the the Writer mutates. A writer should face
//!   minimal contention retrieving this table for mutation since Readers move to
//!   reading the Active table when they are swapped.
//! 
//! The cost of providing no contention to readers, and minimal contention to
//! writers is:
//! 1. Memory - Internally we hold 2 copies of the underlying type the user created.
//!    This is needed to allow there to always be a table that Readers can check out
//!    without contention.
//! 2. Writer thread CPU usage - The writer must apply all updates twice, once to
//!    each table. Lock contention for the writer should be less than with a plain
//!    RwLock due to Readers using the active_table.
//! 
//! The usage is meant to be similar to a RwLock. Instead of multiple threads
//! holding an RwLock though and calling read/write, there is a single Writer that
//! acquire a write guard to the tables, and N Readers which can acquire read guards
//! to the tables. Some of the inspiration came from the left_right crate, so feel
//! free to check that out. We don't implement aliasing, so each table is a true
//! deepcopy of the other. We also don't optimize for startup.
//! 
//! Minimizing lock contention also makes batching a more effective strategy for
//! Reader performance. Now you can grab a ReadGuard, and handle multiple requests
//! without worrying about starving the writer since it will be able to work on the
//! standby table. This means multiple requests can be handled without having to
//! relock the active_table. Similarly you can batch with the Writer without
//! starving the Readers.
//! 
//! Creation is done through the Writer, which can then spawn Readers (Readers
//! are clonable).
//! 
//! We provide 2 modules:
//! 1. primitives - these are building blocks that can be used similarly to a RwLock.
//! 2. collections - these are common collections that use primitives to offer users
//!    an interface very similar to the collections themselves, but conforming to
//!    the requirements of active_standby.

mod read;
mod table;
mod types;
mod write;
pub mod primitives {
    pub use crate::read::{ReadGuard, Reader};
    pub use crate::write::{SendWriteGuard, SendWriter, UpdateTables, WriteGuard, Writer};
}

mod hashmap;
mod vec;
pub mod collections {
    pub use crate::hashmap::hashmap;
    pub use crate::vec::vec;
}
