// Useful:
// - https://doc.rust-lang.org/reference/macros-by-example.html
// - cargo-expand
// - https://stackoverflow.com/a/61189128/7223291.
// - https://doc.rust-lang.org/stable/rustdoc/documentation-tests.html#documenting-macros
// - doc tests mock another crate utilizing the macro.

/// This macro automatically generates an easy to use interface for interacting
/// with an ActiveStandby data structure. The resulting AsLockHandle<T> can be
/// thought of as similar to Arc<RwLock<T>>.
///
/// The user adds mutability to the table is by creating an impl for the
/// generated WriteGuard.
///
/// ```
/// # #[macro_use] extern crate active_standby;
/// pub mod aslock {
///     # use active_standby::primitives::UpdateTables;
///     // The macro can't handle paths, so you can't pass 'std::collections::HashMap'.
///     // In such a case just put 'use std::collections::HashMap' right before the
///     // macro invocation.
///     active_standby::generate_aslock_handle!(i32);
///
///     impl<'w> WriteGuard<'w> {
///         pub fn add_one(&mut self) {
///            struct AddOne {}
///
///            impl<'a> UpdateTables<'a, i32, ()> for AddOne {
///                fn apply_first(&mut self, table: &'a mut i32) {
///                    *table = *table + 1;
///                 }
///                 fn apply_second(mut self, table: &mut i32) {
///                     self.apply_first(table);
///                 }
///             }
///
///             self.guard.update_tables(AddOne {})
///         }
///     }
/// }
///
/// fn main() {
///   let mut aslock = aslock::AsLockHandle::default();
///   assert_eq!(*aslock.read(), 0);
///   {
///     let mut wg = aslock.write();
///     wg.add_one();
///   }
///   assert_eq!(*aslock.read(), 1);
/// }
/// ```
///
/// For a simple example check out bench.rs. For larger examples, check out
/// active_standby::collections.
#[macro_export]
macro_rules! generate_aslock_handle {
    ( $Table:ident
        // Table might be a template type.
        $(<
            // Any number of inner types.
            $( $Inner:tt ),*
        >)?
    ) => {
        struct Writer$(< $($Inner),* >)? {
            writer: $crate::primitives::SyncWriter<$Table $(< $($Inner),* >)? >,
        }

        impl$(< $($Inner),* >)? Writer$(< $($Inner),* >)? {
            pub fn from_identical(
                t1: $Table $(< $($Inner),* >)?,
                t2: $Table $(< $($Inner),* >)?
            ) -> Writer$(< $($Inner),* >)? {
                Writer {
                    writer: $crate::primitives::SyncWriter::from_identical(t1, t2),
                }
            }

            pub fn write(&self) -> WriteGuard<'_, $($($Inner),*)?> {
                WriteGuard {
                    guard: self.writer.write(),
                }
            }

            pub fn new_reader(&self) -> $crate::primitives::Reader<$Table $(< $($Inner),* >)?> {
                self.writer.new_reader()
            }
        }

        pub struct WriteGuard<'w, $($($Inner),*)?> {
            guard: $crate::primitives::SyncWriteGuard<'w, $Table $(< $($Inner),* >)?>,
        }

        impl<'w, $($($Inner),*)?> std::ops::Deref for WriteGuard<'w, $($($Inner),*)?> {
            type Target = $Table$(< $($Inner),* >)?;
            fn deref(&self) -> &Self::Target {
                &*self.guard
            }
        }

        pub struct AsLockHandle$(<$($Inner),*>)? {
            writer: std::sync::Arc<Writer$(<$($Inner),*>)?>,
            reader: $crate::primitives::Reader<$Table$(<$($Inner),*>)?>,
        }

        impl$(<$($Inner),*>)? AsLockHandle$(<$($Inner),*>)?
            where $Table$(<$($Inner),*>)? : Default,
        {
            pub fn default() -> AsLockHandle$(<$($Inner),*>)? {
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

            // Mutable because we do not want AsLock being shared between threads.
            // Clone a new lock to be sent to the other thread.
            pub fn write(&mut self) -> WriteGuard<'_, $($($Inner),*)?>  {
                self.writer.write()
            }

            // Mutable because we do not want AsLock being shared between threads.
            // Clone a new lock to be sent to the other thread.
            pub fn read(&mut self) -> $crate::primitives::ReadGuard<'_, $Table$(<$($Inner),*>)?> {
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
    }
}
