//! A concurrency primitive for high concurrency reads with a single writer.
//!
//! While Readers will need to acquire a reader lock, they are guaranteed to
//! never compete with the writer, meaning readers never face lock contention.
//! This is achieved at the cost of:
//!
//! 1. Memory - Internally we hold 2 copies of the underlying type the user is
//!    using. This allows us to have the active table, which is used by readers
//!    and the standby_table which is used by the writer.
//! 2. Writer thread CPU usage - The writer must apply all updates twice. Lock
//!    contention for the writer should be less than with a plain RwLock due to
//!    Readers using the active_table.
//!
//! The usage is meant to be similar to a RwLock. Some of the inspiration came
//! from the left_right crate, so feel free to check that out. We don't
//! implement aliasing, so each table is a true deepcopy of the other. We also
//! don't optimize for startup.
//!
//! Minimizing lock contention also makes batching a more effective strategy for
//! Reader performance. Now you can grab a ReadGuard, and handle multiple
//! requests without worrying aboutstarving the writer since it will be able to
//! work on the standby table. This means multiple requests can be handled
//! without having to relock the active_table.
//!
//! Creation is done through the Writer, which can then spawn Readers (Readers
//! are clonable).

mod read;
mod table;
mod types;
mod write;
pub mod primitives {
    pub use crate::read::{ReadGuard, Reader};
    pub use crate::write::{SendWriteGuard, SendWriter, UpdateTables, WriteGuard, Writer};
}

mod vec;
pub mod collections {
    pub use crate::vec::vec;
}
