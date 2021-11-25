/// Implementation of BTreeMap for use in the active_standby model.
/// btreemap::AsLockHandle<K, V>, should function similarly to
/// Arc<RwLock<BK, VreeMap<K, V>>>.
use crate::primitives::UpdateTables;
use std::collections::BTreeMap;

struct Insert<K, V> {
    key: K,
    value: V,
}

impl<'a, K, V> UpdateTables<'a, BTreeMap<K, V>, Option<V>> for Insert<K, V>
where
    K: Ord + Clone,
    V: Clone,
{
    fn apply_first(&mut self, table: &'a mut BTreeMap<K, V>) -> Option<V> {
        table.insert(self.key.clone(), self.value.clone())
    }
    fn apply_second(self, table: &mut BTreeMap<K, V>) {
        // Move the value instead of cloning.
        table.insert(self.key, self.value);
    }
}

struct Append<K, V> {
    other: BTreeMap<K, V>,
}

impl<'a, K, V> UpdateTables<'a, BTreeMap<K, V>, ()> for Append<K, V>
where
    K: Ord + Clone,
    V: Clone,
{
    fn apply_first(&mut self, table: &'a mut BTreeMap<K, V>) {
        for (k, v) in self.other.iter() {
            table.insert(k.clone(), v.clone());
        }
    }
    fn apply_second(mut self, table: &mut BTreeMap<K, V>) {
        table.append(&mut self.other)
    }
}

pub mod lockless {
    use super::*;
    crate::generate_lockless_aslockhandle!(BTreeMap<K, V>);

    impl<'w, K, V> WriteGuard<'w, K, V>
    where
        K: 'static + Ord + Clone + Send,
        V: 'static + Clone + Send,
    {
        pub fn insert(&mut self, key: K, value: V) -> Option<V> {
            self.guard.update_tables(Insert { key, value })
        }

        pub fn clear(&mut self) {
            self.guard.update_tables_closure(move |table| table.clear())
        }

        pub fn remove<Q>(&mut self, key_like: Q) -> Option<V>
        where
            K: Ord + std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove(&key_like))
        }

        pub fn remove_entry<Q>(&mut self, key_like: Q) -> Option<(K, V)>
        where
            K: std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove_entry(&key_like))
        }

        pub fn append(&mut self, other: BTreeMap<K, V>) {
            self.guard.update_tables(Append { other })
        }
    }
}

pub mod shared {
    use super::*;
    crate::generate_shared_aslock!(BTreeMap<K, V>);

    impl<'w, K, V> WriteGuard<'w, K, V>
    where
        K: 'static + Ord + Clone + Send,
        V: 'static + Clone + Send,
    {
        pub fn insert(&mut self, key: K, value: V) -> Option<V> {
            self.guard.update_tables(Insert { key, value })
        }

        pub fn clear(&mut self) {
            self.guard.update_tables_closure(move |table| table.clear())
        }

        pub fn remove<Q>(&mut self, key_like: Q) -> Option<V>
        where
            K: Ord + std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove(&key_like))
        }

        pub fn remove_entry<Q>(&mut self, key_like: Q) -> Option<(K, V)>
        where
            K: std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove_entry(&key_like))
        }

        pub fn append(&mut self, other: BTreeMap<K, V>) {
            self.guard.update_tables(Append { other })
        }
    }
}

#[cfg(test)]
mod lockless_test {
    use super::*;
    use maplit::*;

    #[test]
    fn insert() {
        let expected = btreemap! {
            "hello" => 1,
            "world" => 2,
        };

        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn clear() {
        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            wg.clear();
        }

        assert!(table.read().is_empty());
        assert!(table.write().is_empty());
        assert!(table.read().is_empty());
    }

    #[test]
    fn remove() {
        let expected = btreemap! {
            "hello" => 1,
        };

        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove("world"), Some(2));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn remove_entry() {
        let expected = btreemap! {
            "hello" => 1,
        };

        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove_entry("world"), Some(("world", 2)));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn append() {
        let expected = btreemap! {
            "hello" => 1,
            "world" => 2,
            "name's" => 3,
            "joe" => 4,
        };

        println!("1");
        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let map1 = btreemap! {
                "hello" => 1,
                "world" => 2,
            };
            let map2 = btreemap! {
                "name's" => 3,
                "joe" => 4,
            };
            println!("2");
            let mut wg = table.write();
            println!("3");
            wg.append(map1);
            println!("4");
            wg.append(map2);
            println!("5");
            assert_eq!(*wg, expected);
            println!("6");
        }

        println!("7");
        assert_eq!(*table.read(), expected);
        println!("8");
        assert_eq!(*table.write(), expected);
        println!("9");
        assert_eq!(*table.read(), expected);
        println!("10");
    }

    #[test]
    fn debug_str() {
        let table = lockless::AsLockHandle::<i32, i32>::default();
        {
            table.write().insert(12, -1);
        }

        assert_eq!(
            format!("{:?}", table),
            "AsLockHandle { writer: Writer { num_ops_to_replay: 1 }, reader: Reader { num_readers: 1 } }",
        );
        assert_eq!(
            format!("{:?}", table.write()),
            "WriteGuard { num_ops_to_replay: 0, standby_table: TableWriteGuard { standby_table: {12: -1} } }",
        );
        assert_eq!(
            format!("{:?}", table.read()),
            "ReadGuard { active_table: {12: -1} }",
        );
    }
}

#[cfg(test)]
mod shared_test {
    use super::*;
    use maplit::*;
    use std::sync::Arc;

    #[test]
    fn insert() {
        let expected = btreemap! {
            "hello" => 1,
            "world" => 2,
        };

        let table = Arc::new(shared::AsLock::<&str, i32>::default());
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn clear() {
        let table = Arc::new(shared::AsLock::<&str, i32>::default());
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            wg.clear();
        }

        assert!(table.read().is_empty());
        assert!(table.write().is_empty());
        assert!(table.read().is_empty());
    }

    #[test]
    fn remove() {
        let expected = btreemap! {
            "hello" => 1,
        };

        let table = Arc::new(shared::AsLock::<&str, i32>::default());
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove("world"), Some(2));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn remove_entry() {
        let expected = btreemap! {
            "hello" => 1,
        };

        let table = Arc::new(shared::AsLock::<&str, i32>::default());
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove_entry("world"), Some(("world", 2)));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn append() {
        let expected = btreemap! {
            "hello" => 1,
            "world" => 2,
            "name's" => 3,
            "joe" => 4,
        };

        let table = Arc::new(shared::AsLock::<&str, i32>::default());
        {
            let map1 = btreemap! {
                "hello" => 1,
                "world" => 2,
            };
            let map2 = btreemap! {
                "name's" => 3,
                "joe" => 4,
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

    #[test]
    fn debug_str() {
        let table = Arc::new(shared::AsLock::<i32, i32>::default());
        {
            table.write().insert(12, -1);
        }

        assert_eq!(format!("{:?}", table), "AsLock { num_ops_to_replay: 1 }",);
        assert_eq!(
            format!("{:?}", table.write()),
            "WriteGuard { num_ops_to_replay: 0, standby_table: TableWriteGuard { standby_table: {12: -1} } }",
        );
        assert_eq!(
            format!("{:?}", table.read()),
            "ShardedLockReadGuard { lock: ShardedLock { data: {12: -1} } }",
        );
    }
}
