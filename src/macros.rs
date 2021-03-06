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
/// The macro can't handle paths, so you can't pass 'std::collections::HashMap'.
/// In such a case just put 'use std::collections::HashMap' right before the
/// macro invocation.
///
/// For a simple example check out bench.rs. For larger examples, check out
/// active_standby::collections.
///
/// ```
/// pub mod aslock {
///     use active_standby::primitives::UpdateTables;
///
///     // Generate an AsLockHandle, which will give wait free read accees
///     // to the underlying data. This also generates the associated WriteGuard
///     // which is used to mutate the data. Users should interact with this
///     // similarly to Arc<RwLock<i32>>.
///     active_standby::generate_aslock_handle!(i32);
///
///     // Client's must implement the mutable interface that they want to offer
///     // users of their active standby data structure. This is not automatically
///     // generated.
///     impl<'w> WriteGuard<'w> {
///         pub fn add_one(&mut self) {
///             struct AddOne {}
///             
///             impl<'a> UpdateTables<'a, i32, ()> for AddOne {
///                 fn apply_first(&mut self, table: &'a mut i32) {
///                     *table = *table + 1;
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
///     let mut table = aslock::AsLockHandle::new(0);
///     let mut table2 = table.clone();
///     let handle = std::thread::spawn(move || {
///         while *table2.read() != 1 {
///             std::thread::sleep(std::time::Duration::from_micros(100));
///         }
///     });
///
///     {
///         let mut wg = table.write();
///         wg.add_one();
///     }
///     handle.join();
/// }
/// ```
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
            pub fn write(&self) -> WriteGuard<'_, $($($Inner),*)?>  {
                self.writer.write()
            }

            // Mutable because we do not want AsLock being shared between threads.
            // Clone a new lock to be sent to the other thread.
            pub fn read(&self) -> $crate::primitives::ReadGuard<'_, $Table$(<$($Inner),*>)?> {
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
