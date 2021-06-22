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
/// # use active_standby::primitives::UpdateTables;
///
/// // The macro can't handle paths, so you can't pass 'std::collections::HashMap'.
/// use std::collections::HashMap;
/// generate_aslock_handle!(HashMap<K, V>);
///
/// struct Insert<K, V> {
///     key: K,
///     value: V,
/// }
///
/// impl<'a, K, V> UpdateTables<'a, HashMap<K, V>, Option<V>> for Insert<K, V>
/// where
///     K: Eq + std::hash::Hash + Clone,
///     V: Clone,
/// {
///     fn apply_first(&mut self, table: &'a mut HashMap<K, V>) -> Option<V> {
///         table.insert(self.key.clone(), self.value.clone())
///     }
///     fn apply_second(self, table: &mut HashMap<K, V>) {
///         // Move the value instead of cloning.
///         table.insert(self.key, self.value);
///     }
/// }
///
/// impl<'w, K, V> WriteGuard<'w, K, V>
/// where
///     K: 'static + Eq + std::hash::Hash + Clone + Send,
///     V: 'static + Clone + Send,
/// {
///     pub fn insert(&mut self, key: K, value: V) -> Option<V> {
///         self.guard.update_tables(Insert { key, value })
///     }
/// }
///
/// fn main() {
///   let mut aslock = AsLockHandle::<i32, i32>::default();
///   {
///     let mut wg = aslock.write();
///     wg.insert(10, 4);
///   }
///   assert_eq!(aslock.read()[&10], 4);
/// }
/// ```
///
/// For more examples, check out active_standby::collections.
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

        impl$(< $($Inner),* >)? Writer$(< $($Inner),* >)?
            where $Table$(<$($Inner),*>)? : Default,
        {
            #[allow(dead_code)]
            pub fn default() -> Writer$(< $($Inner),* >)? {
                Self::from_identical($Table::default(), $Table::default())
            }
        }

        impl$(< $($Inner),* >)? Writer$(< $($Inner),* >)?
            where $Table$(<$($Inner),*>)? : Clone,
        {
            #[allow(dead_code)]
            pub fn new(t: $Table $(< $($Inner),* >)?) -> Writer$(< $($Inner),* >)? {
                Self::from_identical(t.clone(), t)
            }
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
