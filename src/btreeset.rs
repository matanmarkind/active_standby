/// Implementation of BTreeSet for use in the active_standby model.
/// btreeset::AsLockHandle<T>, should function similarly to
/// Arc<RwLock<BTreeSet<T>>>.
pub mod btreeset {
    use crate::primitives::UpdateTables;
    use std::collections::BTreeSet;

    crate::generate_aslock_handle!(BTreeSet<T>);

    struct Insert<T> {
        value: T,
    }

    impl<'a, T> UpdateTables<'a, BTreeSet<T>, bool> for Insert<T>
    where
        T: Ord + Clone,
    {
        fn apply_first(&mut self, table: &'a mut BTreeSet<T>) -> bool {
            table.insert(self.value.clone())
        }
        fn apply_second(self, table: &mut BTreeSet<T>) {
            // Move the value instead of cloning.
            table.insert(self.value);
        }
    }

    struct Replace<T> {
        value: T,
    }

    impl<'a, T> UpdateTables<'a, BTreeSet<T>, Option<T>> for Replace<T>
    where
        T: Ord + Clone,
    {
        fn apply_first(&mut self, table: &'a mut BTreeSet<T>) -> Option<T> {
            table.replace(self.value.clone())
        }
        fn apply_second(self, table: &mut BTreeSet<T>) {
            // Move the value instead of cloning.
            table.replace(self.value);
        }
    }

    struct Clear {}

    impl<'a, T> UpdateTables<'a, BTreeSet<T>, ()> for Clear
    where
        T: Ord + Clone,
    {
        fn apply_first(&mut self, table: &'a mut BTreeSet<T>) {
            table.clear()
        }
        fn apply_second(mut self, table: &mut BTreeSet<T>) {
            self.apply_first(table);
        }
    }

    struct Remove<Q> {
        value_like: Q,
    }

    impl<'a, T, Q> UpdateTables<'a, BTreeSet<T>, bool> for Remove<Q>
    where
        Q: Ord,
        T: Ord + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &'a mut BTreeSet<T>) -> bool {
            table.remove(&self.value_like)
        }
        fn apply_second(mut self, table: &mut BTreeSet<T>) {
            self.apply_first(table);
        }
    }

    struct Take<Q> {
        value_like: Q,
    }

    impl<'a, T, Q> UpdateTables<'a, BTreeSet<T>, Option<T>> for Take<Q>
    where
        Q: Ord,
        T: Ord + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &'a mut BTreeSet<T>) -> Option<T> {
            table.take(&self.value_like)
        }
        fn apply_second(mut self, table: &mut BTreeSet<T>) {
            self.apply_first(table);
        }
    }

    struct Append<T> {
        other: BTreeSet<T>,
    }

    impl<'a, T> UpdateTables<'a, BTreeSet<T>, ()> for Append<T>
    where
        T: Ord + Clone,
    {
        fn apply_first(&mut self, table: &'a mut BTreeSet<T>) {
            for k in self.other.iter() {
                table.insert(k.clone());
            }
        }
        fn apply_second(mut self, table: &mut BTreeSet<T>) {
            table.append(&mut self.other);
        }
    }

    impl<'w, T> WriteGuard<'w, T>
    where
        T: 'static + Ord + Clone + Send,
    {
        pub fn insert(&mut self, value: T) -> bool {
            self.guard.update_tables(Insert { value })
        }

        pub fn replace(&mut self, value: T) -> Option<T> {
            self.guard.update_tables(Replace { value })
        }

        pub fn clear(&mut self) {
            self.guard.update_tables(Clear {})
        }

        pub fn remove<Q>(&mut self, value_like: Q) -> bool
        where
            T: std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard.update_tables(Remove { value_like })
        }

        pub fn take<Q>(&mut self, value_like: Q) -> Option<T>
        where
            T: std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard.update_tables(Take { value_like })
        }

        pub fn append(&mut self, other: BTreeSet<T>) {
            self.guard.update_tables(Append { other })
        }
    }
}

#[cfg(test)]
mod test {
    use super::btreeset::*;
    use maplit::*;

    #[test]
    fn insert() {
        let expected = btreeset! {
            "hello",
            "world",
        };

        let table = AsLockHandle::<&str>::default();
        {
            let mut wg = table.write();
            wg.insert("hello");
            wg.insert("world");
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
        let expected = btreeset! {
            "hello",
        };
        let table = AsLockHandle::<&str>::default();
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
    fn append() {
        let expected = btreeset! {
            "hello",
            "world",
            "name's",
            "joe",
        };

        let table = AsLockHandle::<&str>::default();
        {
            let map1 = btreeset! {
                "hello",
                "world",
            };
            let map2 = btreeset! {
                "name's" ,
                "joe" ,
            };
            let mut wg = table.write();
            wg.append(map1);
            wg.append(map2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }
}
