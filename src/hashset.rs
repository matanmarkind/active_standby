/// Implementation of HashSet for use in the active_standby model.
/// hashset::AsLockHandle<T>, should function similarly to
/// Arc<RwLock<HashSet<T>>>.
pub mod hashset {
    use crate::primitives::UpdateTables;
    use std::borrow::Borrow;
    use std::collections::HashSet;
    use std::hash::Hash;

    crate::generate_aslock_handle!(HashSet<T>);

    impl<'w, 'a, T> WriteGuard<'w, T>
    where
        T: 'static + Eq + Hash + Clone + Send,
    {
        pub fn clear(&mut self) {
            struct Clear {}
            impl<'a, T> UpdateTables<'a, HashSet<T>, ()> for Clear {
                fn apply_first(&mut self, table: &'a mut HashSet<T>) {
                    table.clear()
                }
                fn apply_second(mut self, table: &mut HashSet<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(Clear {})
        }

        pub fn shrink_to_fit(&mut self) {
            struct ShrinkToFit {}
            impl<'a, T> UpdateTables<'a, HashSet<T>, ()> for ShrinkToFit
            where
                T: Eq + Hash,
            {
                fn apply_first(&mut self, table: &'a mut HashSet<T>) {
                    table.shrink_to_fit()
                }
                fn apply_second(mut self, table: &mut HashSet<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(ShrinkToFit {})
        }

        pub fn reserve(&mut self, additional: usize) {
            struct Reserve {
                additional: usize,
            }
            impl<'a, T> UpdateTables<'a, HashSet<T>, ()> for Reserve
            where
                T: Eq + Hash,
            {
                fn apply_first(&mut self, table: &'a mut HashSet<T>) {
                    table.reserve(self.additional)
                }
                fn apply_second(mut self, table: &mut HashSet<T>) {
                    self.apply_first(table);
                }
            }
            self.guard.update_tables(Reserve { additional })
        }

        pub fn insert(&mut self, value: T) -> bool {
            struct Insert<T> {
                value: T,
            }
            impl<'a, T> UpdateTables<'a, HashSet<T>, bool> for Insert<T>
            where
                T: Eq + Hash + Clone,
            {
                fn apply_first(&mut self, table: &'a mut HashSet<T>) -> bool {
                    table.insert(self.value.clone())
                }
                fn apply_second(self, table: &mut HashSet<T>) {
                    // Move the value instead of cloning.
                    table.insert(self.value);
                }
            }

            self.guard.update_tables(Insert { value })
        }

        pub fn replace(&mut self, value: T) -> Option<T> {
            struct Replace<T> {
                value: T,
            }
            impl<'a, T> UpdateTables<'a, HashSet<T>, Option<T>> for Replace<T>
            where
                T: Eq + Hash + Clone,
            {
                fn apply_first(&mut self, table: &'a mut HashSet<T>) -> Option<T> {
                    table.replace(self.value.clone())
                }
                fn apply_second(self, table: &mut HashSet<T>) {
                    // Move the value instead of cloning.
                    table.replace(self.value);
                }
            }

            self.guard.update_tables(Replace { value })
        }

        pub fn remove<Q>(&mut self, value_like: Q) -> bool
        where
            T: Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            struct Remove<Q> {
                value_like: Q,
            }

            impl<'a, T, Q> UpdateTables<'a, HashSet<T>, bool> for Remove<Q>
            where
                Q: Eq + Hash,
                T: Eq + Hash + Borrow<Q>,
            {
                fn apply_first(&mut self, table: &'a mut HashSet<T>) -> bool {
                    table.remove(&self.value_like)
                }
                fn apply_second(mut self, table: &mut HashSet<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(Remove { value_like })
        }

        pub fn take<Q>(&mut self, value_like: Q) -> Option<T>
        where
            T: Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            struct Take<Q> {
                value_like: Q,
            }

            impl<'a, T, Q> UpdateTables<'a, HashSet<T>, Option<T>> for Take<Q>
            where
                Q: Eq + Hash,
                T: Eq + Hash + Borrow<Q>,
            {
                fn apply_first(&mut self, table: &'a mut HashSet<T>) -> Option<T> {
                    table.take(&self.value_like)
                }
                fn apply_second(mut self, table: &mut HashSet<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(Take { value_like })
        }

        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Send + Clone + FnMut(&T) -> bool,
        {
            struct Retain<T, F>
            where
                F: 'static + Clone + FnMut(&T) -> bool,
            {
                f: F,
                _compile_k_v: std::marker::PhantomData<T>,
            }

            impl<'a, T, F> UpdateTables<'a, HashSet<T>, ()> for Retain<T, F>
            where
                T: Eq + Hash,
                F: 'static + Clone + FnMut(&T) -> bool,
            {
                fn apply_first(&mut self, table: &'a mut HashSet<T>) {
                    table.retain(self.f.clone())
                }

                fn apply_second(self, table: &mut HashSet<T>) {
                    table.retain(self.f)
                }
            }

            self.guard.update_tables(Retain {
                f,
                _compile_k_v: std::marker::PhantomData,
            })
        }

        pub fn drain(&'a mut self) -> std::collections::hash_set::Drain<'a, T> {
            struct Drain {}

            impl<'a, T> UpdateTables<'a, HashSet<T>, std::collections::hash_set::Drain<'a, T>> for Drain {
                fn apply_first(
                    &mut self,
                    table: &'a mut HashSet<T>,
                ) -> std::collections::hash_set::Drain<'a, T> {
                    table.drain()
                }

                fn apply_second(mut self, table: &mut HashSet<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(Drain {})
        }
    }
}

#[cfg(test)]
mod test {
    use super::hashset::*;
    use maplit::*;
    use more_asserts::*;

    #[test]
    fn insert_and_replace() {
        let expected = hashset! {
            "hello",
            "world",
        };

        let table = AsLockHandle::<&str>::default();
        {
            let mut wg = table.write();
            wg.insert("hello");
            wg.insert("world");
            wg.replace("world");
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn clear() {
        let table = AsLockHandle::<&str>::default();
        {
            let mut wg = table.write();
            wg.insert("hello");
            wg.insert("world");
            wg.clear();
        }

        assert!(table.read().is_empty());
        assert!(table.write().is_empty());
        assert!(table.read().is_empty());
    }

    #[test]
    fn remove_and_take() {
        let expected = hashset! {
            "hello",
        };

        let table = AsLockHandle::<&str>::new(std::collections::HashSet::new());
        {
            let mut wg = table.write();
            wg.insert("hello");
            wg.insert("world");
            wg.insert("I");
            assert_eq!(wg.remove("world"), true);
            assert_eq!(wg.take("I"), Some("I"));
            assert_eq!(wg.take("I"), None);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn shrink_to_fit_and_reserve() {
        let table = AsLockHandle::<&str>::from_identical(
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        );
        let initial_capacity;
        let additional = 10;
        {
            let mut wg = table.write();
            wg.insert("hello");
            wg.insert("world");
            wg.shrink_to_fit();
            initial_capacity = wg.capacity();
            wg.reserve(additional);
            assert_ge!(wg.capacity(), initial_capacity + additional);
        }

        assert_ge!(table.read().capacity(), initial_capacity + additional);
        assert_ge!(table.write().capacity(), initial_capacity + additional);
        assert_ge!(table.read().capacity(), initial_capacity + additional);
    }

    #[test]
    fn retain() {
        let expected = hashset! {
            "joe",
            "world",
            "hello",
            "name",
        };
        let table = AsLockHandle::<&str>::default();
        {
            let mut wg = table.write();
            wg.insert("hello");
            wg.insert("world");
            wg.insert("my");
            wg.insert("name");
            wg.insert("is");
            wg.insert("joe");
            wg.retain(|&k| k.len() > 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn drain() {
        let expected = hashset! {
            "hello" ,
            "world",
        };

        let table = AsLockHandle::<&str>::default();
        {
            let mut wg = table.write();
            wg.insert("hello");
            wg.insert("world");
            assert_eq!(*wg, expected);
            assert_eq!(
                wg.drain().collect::<std::collections::HashSet<_>>(),
                expected
            );
        }

        assert!(table.read().is_empty());
        assert!(table.write().is_empty());
        assert!(table.read().is_empty());
    }
}
