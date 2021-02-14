// TODO: Consider using crossbeam-utils sharded RwLock since it's optimized for fast
// reads. Since reads should never be contested a faster read implementation
// seems good. The slower write lock shouldn't be an issue since the slowness on
// writes that I am worried about is due to reader threads still holding the new
// 'standby_table' when we try to create a new WriteGuard.

// Define locally the lock types used incase we want to switch to a different
// implementation.
pub type RwLock<T> = std::sync::RwLock<T>;
pub type RwLockWriteGuard<'w, T> = std::sync::RwLockWriteGuard<'w, T>;
pub type RwLockReadGuard<'r, T> = std::sync::RwLockReadGuard<'r, T>;
