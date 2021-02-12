/// Simplified version of the left_right crate.
///
/// This crate trades out some performance and memory optimizations in that
/// crate for simplicity. Specifically:
/// - Aliasing - we don't alias elements, instead we deepcopy them so each table
///   has full ownership over its contents.
/// - First update - we don't provide a special interface for updating the table
///   at the beginning before there are any readers.
///
/// Reads should function like using a plain RwLock, but there will never be
/// contention with a writer.

// Private modules.
mod table;

// Public modules.
pub mod read;
pub mod write;
