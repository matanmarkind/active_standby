// Useful:
// - https://doc.rust-lang.org/reference/macros-by-example.html
// - cargo-expand
// - https://stackoverflow.com/a/61189128/7223291.
// - https://doc.rust-lang.org/stable/rustdoc/documentation-tests.html#documenting-macros
// - doc tests mock another crate utilizing the macro.

/// These macro automatically generates an easy to use interface for interacting
/// with an ActiveStandby data structure. Note that this is done for each
/// underlying table, as opposed to an RwLock which is generic over all
/// underlying types. This is because there are really 2 underlying tables which
/// need to be kept in sync. The client adds mutability to the table is by
/// creating an impl for the generated WriteGuard.
///
/// This macro is valid for templated types, it doesn't have to be concrete. The
/// macro can't handle paths, so you can't pass 'std::collections::HashMap'. In
/// such a case just put 'use std::collections::HashMap' right before the macro
/// invocation.
///
/// For a simple example check out crate level docs or bench.rs. For larger
/// examples, check out active_standby::collections.

/// Generates an AsLockHandle for the type passed in. This follows the lockless
/// model, meaning that reads don't perform synchronization, but that the
/// resultant AsLockHandle cannot be shared across threads. Though it can be
/// cloned and sent across threads.
#[macro_export]
macro_rules! generate_lockless_aslockhandle {
    ( $Table:ident
        // Table might be a template type.
        $(<
            // Any number of inner types.
            $( $Inner:tt ),*
        >)?
    ) => {
        struct Writer$(< $($Inner),* >)? {
            writer: $crate::primitives::lockless::SyncWriter<$Table $(< $($Inner),* >)? >,
        }

        impl$(< $($Inner),* >)? Writer$(< $($Inner),* >)? {
            pub fn from_identical(
                t1: $Table $(< $($Inner),* >)?,
                t2: $Table $(< $($Inner),* >)?
            ) -> Writer$(< $($Inner),* >)? {
                Writer {
                    writer: $crate::primitives::lockless::SyncWriter::from_identical(t1, t2),
                }
            }

            pub fn write(&self) -> WriteGuard<'_, $($($Inner),*)?> {
                WriteGuard {
                    guard: self.writer.write(),
                }
            }

            pub fn new_reader(&self) -> $crate::primitives::lockless::Reader<$Table $(< $($Inner),* >)?> {
                self.writer.new_reader()
            }
        }

        pub struct WriteGuard<'w, $($($Inner),*)?> {
            guard: $crate::primitives::lockless::SyncWriteGuard<'w, $Table $(< $($Inner),* >)?>,
        }

        impl<'w, $($($Inner),*)?> std::ops::Deref for WriteGuard<'w, $($($Inner),*)?> {
            type Target = $Table$(< $($Inner),* >)?;
            fn deref(&self) -> &Self::Target {
                &*self.guard
            }
        }

        pub struct AsLockHandle$(<$($Inner),*>)? {
            writer: std::sync::Arc<Writer$(<$($Inner),*>)?>,
            reader: $crate::primitives::lockless::Reader<$Table$(<$($Inner),*>)?>,
        }

        impl$(<$($Inner),*>)? Default for AsLockHandle$(<$($Inner),*>)?
            where $Table$(<$($Inner),*>)? : Default,
        {
            fn default() -> AsLockHandle$(<$($Inner),*>)? {
                Self::from_identical($Table::default(), $Table::default())
            }
        }

        impl$(<$($Inner),*>)? AsLockHandle$(<$($Inner),*>)?
            where $Table$(<$($Inner),*>)? : Clone,
        {
            pub fn new(t: $Table $(< $($Inner),* >)?) -> AsLockHandle$(<$($Inner),*>)? {
                Self::from_identical(t.clone(), t)
            }
        }

        impl$(<$($Inner),*>)? AsLockHandle$(<$($Inner),*>)? {
            pub fn from_identical(
                t1: $Table $(< $($Inner),* >)?,
                t2: $Table $(< $($Inner),* >)?
            ) -> AsLockHandle$(<$($Inner),*>)? {
                let writer = std::sync::Arc::new(Writer::from_identical(t1, t2));
                let reader = writer.new_reader();
                AsLockHandle { writer, reader }
            }

            pub fn write(&self) -> WriteGuard<'_, $($($Inner),*)?>  {
                self.writer.write()
            }

            pub fn read(&self) -> $crate::primitives::lockless::ReadGuard<'_, $Table$(<$($Inner),*>)?> {
                self.reader.read()
            }
        }

        impl$(<$($Inner),*>)? Clone for AsLockHandle$(<$($Inner),*>)? {
            fn clone(&self) -> AsLockHandle$(<$($Inner),*>)? {
                let writer = std::sync::Arc::clone(&self.writer);
                let reader = writer.new_reader();
                AsLockHandle { writer, reader }
            }
        }

        impl$(<$($Inner),*>)? std::fmt::Debug for AsLockHandle$(<$($Inner),*>)?
            where $Table$(<$($Inner),*>)? : std::fmt::Debug,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("AsLockHandle")
                    .field("reader", &self.read())
                    .finish()
            }
        }
    }
}

/// Generates an AsLock for the type passed in. This follows the shared model,
/// meaning that you can share this across threads by wrapping it in an Arc like
/// an RwLock.
#[macro_export]
macro_rules! generate_shared_aslock {
    ( $Table:ident
        // Table might be a template type.
        $(<
            // Any number of inner types.
            $( $Inner:tt ),*
        >)?
    ) => {
        pub struct AsLock$(< $($Inner),* >)? {
            lock: $crate::primitives::shared::AsLock<$Table $(< $($Inner),* >)? >,
        }

        impl$(< $($Inner),* >)? AsLock$(< $($Inner),* >)? {
            pub fn from_identical(
                t1: $Table $(< $($Inner),* >)?,
                t2: $Table $(< $($Inner),* >)?
            ) -> AsLock$(< $($Inner),* >)? {
                AsLock {
                    lock: $crate::primitives::shared::AsLock::from_identical(t1, t2),
                }
            }

            pub fn write(&self) -> WriteGuard<'_, $($($Inner),*)?> {
                WriteGuard {
                    guard: self.lock.write(),
                }
            }

            pub fn read(&self) -> $crate::primitives::RwLockReadGuard<'_, $Table $(< $($Inner),* >)?> {
                self.lock.read()
            }
        }

        impl$(<$($Inner),*>)? Default for AsLock$(<$($Inner),*>)?
            where $Table$(<$($Inner),*>)? : Default,
        {
            fn default() -> AsLock$(<$($Inner),*>)? {
                Self::from_identical($Table::default(), $Table::default())
            }
        }

        impl$(<$($Inner),*>)? AsLock$(<$($Inner),*>)?
            where $Table$(<$($Inner),*>)? : Clone,
        {
            pub fn new(t: $Table $(< $($Inner),* >)?) -> AsLock$(<$($Inner),*>)? {
                Self::from_identical(t.clone(), t)
            }
        }

        impl$(<$($Inner),*>)? std::fmt::Debug for AsLock$(<$($Inner),*>)?
            where $Table$(<$($Inner),*>)? : std::fmt::Debug,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("AsLock")
                    .field("reader", &self.read())
                    .finish()
            }
        }

        pub struct WriteGuard<'w, $($($Inner),*)?> {
            guard: $crate::primitives::shared::WriteGuard<'w, $Table $(< $($Inner),* >)?>,
        }

        impl<'w, $($($Inner),*)?> std::ops::Deref for WriteGuard<'w, $($($Inner),*)?> {
            type Target = $Table$(< $($Inner),* >)?;
            fn deref(&self) -> &Self::Target {
                &*self.guard
            }
        }
    }
}
