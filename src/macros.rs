// Useful:
// - https://doc.rust-lang.org/reference/macros-by-example.html
// - cargo-expand
// - https://stackoverflow.com/a/61189128/7223291.
// - https://doc.rust-lang.org/stable/rustdoc/documentation-tests.html#documenting-macros
// - doc tests mock another crate utilizing the macro.

/// These macros automatically generates an easy to use interface for
/// interacting with an ActiveStandby data structure. Note that this is done for
/// each underlying table, as opposed to an RwLock which is generic over all
/// underlying types.
///
/// Using the plain AsLockHandle/AsLock requires users to call to the
/// `update_tables` interface as oppossed to an RwLock which lets users directly
/// mutate their desired table. Using these macros allows client libraries to
/// create interfaces (per container), which wrap the calls to `update_tables`
/// and allows for a UI much like a plain RwLock.
///
/// This macro is valid for templated types, it doesn't have to be concrete. The
/// macro can't handle paths, so you can't pass 'std::collections::HashMap'. In
/// such a case just put the `use` statement right before the macro invocation.
///
/// For a simple example check out crate level docs or bench.rs. For larger
/// examples, check out collections.
///
/// TODO: Is there some way to get all of the mutable methods of Table and
/// automatically generate `UpdateTables` wrappers for them?

/// Generates a lockless::AsLockHandle for the type passed in.
#[macro_export]
macro_rules! generate_lockless_aslockhandle {
    ( $Table:ident
        // Table might be a template type.
        $(<
            // Any number of inner types.
            $( $Inner:tt ),*
        >)?
    ) => {
        // WriteGuard must be a new struct, because clients will implement the
        // update functions for the generated WriteGuard type. If this was just
        // a type alias, clients would be blocked from creating impl blocks
        // outside of the active_standby crate.
        pub struct WriteGuard<'w, $($($Inner),*)?> {
            guard: $crate::primitives::lockless::WriteGuard<'w, $Table $(< $($Inner),* >)?>,
        }

        // Allow the user to `update_tables` directly in case there is an interface missing.
        impl<'w, $($($Inner),*)?> WriteGuard<'w, $($($Inner),*)?> {
            pub fn update_tables<'a, R>(
                &'a mut self,
                update: impl $crate::primitives::UpdateTables<'a, $Table$(< $($Inner),* >)?, R> + 'static + Sized + Send,
            ) -> R {
                self.guard.update_tables(update)
            }

            pub fn update_tables_closure<R>(
                &mut self,
                update: impl Fn(&mut $Table$(< $($Inner),* >)?) -> R + 'static + Sized + Send,
            ) -> R {
                self.guard.update_tables_closure(update)
            }
        }

        // Deref should pass through the wrapper WriteGuard and look like the
        // user holds a primitive WriteGuard to the underlying table.
        impl<'w, $($($Inner),*)?> std::ops::Deref for WriteGuard<'w, $($($Inner),*)?> {
            type Target = $Table$(< $($Inner),* >)?;
            fn deref(&self) -> &Self::Target {
                &*self.guard
            }
        }

        // Debug should pass through the wrapper WriteGuard and look like the
        // user holds a primitive WriteGuard to the underlying table.
        impl<'w, $($($Inner),*)?> std::fmt::Debug for WriteGuard<'w, $($($Inner),*)?>
            where $Table$(<$($Inner),*>)? : std::fmt::Debug,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.guard.fmt(f)
            }
        }

        type AsLockHandleAlias$(< $($Inner),* >)? =
            $crate::primitives::lockless::AsLockHandle<$Table $(< $($Inner),* >)? >;

        // AsLockHandle needs to be a new struct, because we need to "override"
        // the inner call to 'write' so that it will produce the new WriteGuard
        // type that is defined here.
        pub struct AsLockHandle$(< $($Inner),* >)? {
            inner: AsLockHandleAlias$(< $($Inner),* >)?,
        }

        impl$(< $($Inner),* >)? AsLockHandle$(< $($Inner),* >)? {
            pub fn from_identical(
                t1: $Table $(< $($Inner),* >)?,
                t2: $Table $(< $($Inner),* >)?
            ) -> AsLockHandle$(<$($Inner),*>)? {
                AsLockHandle {
                    inner: AsLockHandleAlias::from_identical(t1, t2)
                }
            }

            pub fn write(&self) -> $crate::primitives::LockResult<WriteGuard<'_, $($($Inner),*)?>> {
                // Type conversion from generic WriteGuard to the generated WriteGuard.
                match self.inner.write() {
                    Ok(g) => Ok(WriteGuard {
                        guard: g
                    }),
                    Err(g) => Err(std::sync::PoisonError::new(
                        WriteGuard { guard: g.into_inner() }
                    ))
                }
            }
        }

        impl$(< $($Inner),* >)? AsLockHandle$(< $($Inner),* >)?
        where
            $Table$(<$($Inner),*>)? : Clone,
        {
            pub fn new(t: $Table $(< $($Inner),* >)?) -> AsLockHandle$(<$($Inner),*>)? {
                AsLockHandle {
                    inner: AsLockHandleAlias::from_identical(t.clone(), t)
                }
            }
        }

        impl$(< $($Inner),* >)? Default for AsLockHandle$(< $($Inner),* >)?
        where
            $Table$(<$($Inner),*>)? : Default,
        {
            fn default() -> AsLockHandle$(<$($Inner),*>)? {
                AsLockHandle {
                    inner: AsLockHandleAlias::from_identical($Table::default(), $Table::default())
                }
            }
        }

        impl$(< $($Inner),* >)? std::ops::Deref  for AsLockHandle$(< $($Inner),* >)? {
            type Target = AsLockHandleAlias$(< $($Inner),* >)?;
            fn deref(&self) -> &Self::Target {
                &self.inner
            }
        }

        impl$(< $($Inner),* >)? std::fmt::Debug  for AsLockHandle$(< $($Inner),* >)?
            where $Table$(<$($Inner),*>)? : std::fmt::Debug,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.inner.fmt(f)
            }
        }

        impl$(< $($Inner),* >)? Clone for AsLockHandle$(< $($Inner),* >)? {
            fn clone(&self) -> AsLockHandle$(<$($Inner),*>)? {
                AsLockHandle {
                    inner: self.inner.clone(),
                }
            }
        }
    }
}

/// Generates a shared::AsLock for the type passed in.
#[macro_export]
macro_rules! generate_shared_aslock {
    ( $Table:ident
        // Table might be a template type.
        $(<
            // Any number of inner types.
            $( $Inner:tt ),*
        >)?
    ) => {
        // WriteGuard must be a new struct, because clients will implement the
        // update functions for the generated WriteGuard type. If this was just
        // a type alias, clients would be blocked from creating impl blocks
        // outside of the active_standby crate.
        pub struct WriteGuard<'w, $($($Inner),*)?> {
            guard: $crate::primitives::shared::WriteGuard<'w, $Table $(< $($Inner),* >)?>,
        }

        // Allow the user to `update_tables` directly in case there is an interface missing.
        impl<'w, $($($Inner),*)?> WriteGuard<'w, $($($Inner),*)?> {
            pub fn update_tables<'a, R>(
                &'a mut self,
                update: impl $crate::primitives::UpdateTables<'a, $Table$(< $($Inner),* >)?, R> + 'static + Sized + Send,
            ) -> R {
                self.guard.update_tables(update)
            }

            pub fn update_tables_closure<R>(
                &mut self,
                update: impl Fn(&mut $Table$(< $($Inner),* >)?) -> R + 'static + Sized + Send,
            ) -> R {
                self.guard.update_tables_closure(update)
            }
        }

        // Deref should pass through the wrapper WriteGuard and look like the
        // user holds a primitive WriteGuard to the underlying table.
        impl<'w, $($($Inner),*)?> std::ops::Deref for WriteGuard<'w, $($($Inner),*)?> {
            type Target = $Table$(< $($Inner),* >)?;
            fn deref(&self) -> &Self::Target {
                &*self.guard
            }
        }

        // Debug should pass through the wrapper WriteGuard and look like the
        // user holds a primitive WriteGuard to the underlying table.
        impl<'w, $($($Inner),*)?> std::fmt::Debug for WriteGuard<'w, $($($Inner),*)?>
            where $Table$(<$($Inner),*>)? : std::fmt::Debug,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.guard.fmt(f)
            }
        }

        type AsLockAlias$(< $($Inner),* >)? =
            $crate::primitives::shared::AsLock<$Table $(< $($Inner),* >)? >;

        // AsLock needs to be a new struct, because we need to "override" the
        // inner call to 'write' so that it will produce the new WriteGuard
        // type that is defined here. Note that AsLock is not identical to
        // AsLockHandle. For instance there is no Clone for AsLock, since it is
        // meant to be behind an Arc.
        pub struct AsLock$(< $($Inner),* >)? {
            inner: AsLockAlias$(< $($Inner),* >)?,
        }

        impl$(< $($Inner),* >)? AsLock$(< $($Inner),* >)? {
            pub fn from_identical(
                t1: $Table $(< $($Inner),* >)?,
                t2: $Table $(< $($Inner),* >)?
            ) -> AsLock$(<$($Inner),*>)? {
                AsLock {
                    inner: AsLockAlias::from_identical(t1, t2)
                }
            }

            pub fn write(&self) -> $crate::primitives::LockResult<WriteGuard<'_, $($($Inner),*)?>> {
                match self.inner.write() {
                    Ok(guard) => Ok(WriteGuard {
                        guard
                    }),
                    Err(g) => Err(std::sync::PoisonError::new(
                        WriteGuard { guard: g.into_inner() }
                    ))
                }
            }
        }

        impl$(< $($Inner),* >)? AsLock$(< $($Inner),* >)?
        where
            $Table$(<$($Inner),*>)? : Clone,
        {
            pub fn new(t: $Table $(< $($Inner),* >)?) -> AsLock$(<$($Inner),*>)? {
                AsLock {
                    inner: AsLockAlias::new(t)
                }
            }
        }

        impl$(< $($Inner),* >)? Default for AsLock$(< $($Inner),* >)?
        where
            $Table$(<$($Inner),*>)? : Default,
        {
            fn default() -> AsLock$(<$($Inner),*>)? {
                AsLock {
                    inner: AsLockAlias::default()
                }
            }
        }

        impl$(< $($Inner),* >)? std::ops::Deref  for AsLock$(< $($Inner),* >)? {
            type Target = AsLockAlias$(< $($Inner),* >)?;
            fn deref(&self) -> &Self::Target {
                &self.inner
            }
        }

        impl$(< $($Inner),* >)? std::fmt::Debug  for AsLock$(< $($Inner),* >)?
            where $Table$(<$($Inner),*>)? : std::fmt::Debug,
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.inner.fmt(f)
            }
        }
    }
}
