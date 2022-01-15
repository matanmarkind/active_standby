use crate::primitives::UpdateTables;
use std::collections::HashMap;
use std::hash::Hash;

struct Insert<K, V> {
    key: K,
    value: V,
}

impl<'a, K, V> UpdateTables<'a, HashMap<K, V>, Option<V>> for Insert<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    fn apply_first(&mut self, table: &'a mut HashMap<K, V>) -> Option<V> {
        table.insert(self.key.clone(), self.value.clone())
    }
    fn apply_second(self, table: &mut HashMap<K, V>) {
        // Move the value instead of cloning.
        table.insert(self.key, self.value);
    }
}

struct Drain {}

impl<'a, K, V> UpdateTables<'a, HashMap<K, V>, std::collections::hash_map::Drain<'a, K, V>>
    for Drain
where
    K: Eq + Hash,
{
    fn apply_first(
        &mut self,
        table: &'a mut HashMap<K, V>,
    ) -> std::collections::hash_map::Drain<'a, K, V> {
        table.drain()
    }
    fn apply_second(mut self, table: &mut HashMap<K, V>) {
        self.apply_first(table);
    }
}

struct Retain<K, V, F>
where
    F: 'static + Clone + FnMut(&K, &mut V) -> bool,
{
    f: F,
    _compile_k_v: std::marker::PhantomData<(K, V)>,
}
impl<'a, K, V, F> UpdateTables<'a, HashMap<K, V>, ()> for Retain<K, V, F>
where
    K: Eq + Hash,
    F: 'static + Clone + FnMut(&K, &mut V) -> bool,
{
    fn apply_first(&mut self, table: &'a mut HashMap<K, V>) {
        table.retain(self.f.clone())
    }
    fn apply_second(self, table: &mut HashMap<K, V>) {
        table.retain(self.f)
    }
}

/// Implementation of HashMap for use in the active_standby model.
/// `lockless::AsLockHandle<K, V>`, should function similarly to
/// `Arc<RwLock<HashMap<K, V>>>`.
pub mod lockless {
    use super::*;
    crate::generate_lockless_aslockhandle!(HashMap<K, V>);

    impl<'w, 'a, K, V> WriteGuard<'w, K, V>
    where
        K: 'static + Eq + Hash + Clone + Send,
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
            K: std::borrow::Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove(&key_like))
        }

        pub fn remove_entry<Q>(&mut self, key_like: Q) -> Option<(K, V)>
        where
            K: std::borrow::Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove_entry(&key_like))
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard
                .update_tables_closure(move |table| table.reserve(additional))
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard
                .update_tables_closure(move |table| table.shrink_to_fit())
        }

        pub fn drain(&'a mut self) -> std::collections::hash_map::Drain<'a, K, V> {
            self.guard.update_tables(Drain {})
        }

        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Send + Clone + FnMut(&K, &mut V) -> bool,
        {
            self.guard.update_tables(Retain {
                f,
                _compile_k_v: std::marker::PhantomData,
            })
        }
    }
}

/// Implementation of HashMap for use in the active_standby model.
/// `shared::AsLock<K, V>`, should function similarly to `RwLock<HashMap<K,
/// V>>`.
pub mod shared {
    use super::*;
    crate::generate_shared_aslock!(HashMap<K, V>);

    impl<'w, 'a, K, V> WriteGuard<'w, K, V>
    where
        K: 'static + Eq + Hash + Clone + Send,
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
            K: std::borrow::Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove(&key_like))
        }

        pub fn remove_entry<Q>(&mut self, key_like: Q) -> Option<(K, V)>
        where
            K: std::borrow::Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove_entry(&key_like))
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard
                .update_tables_closure(move |table| table.reserve(additional))
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard
                .update_tables_closure(move |table| table.shrink_to_fit())
        }

        pub fn drain(&'a mut self) -> std::collections::hash_map::Drain<'a, K, V> {
            self.guard.update_tables(Drain {})
        }

        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Send + Clone + FnMut(&K, &mut V) -> bool,
        {
            self.guard.update_tables(Retain {
                f,
                _compile_k_v: std::marker::PhantomData,
            })
        }
    }
}

#[cfg(test)]
mod lockless_test {
    use super::*;
    use maplit::*;
    use more_asserts::*;

    #[test]
    fn insert() {
        let expected = hashmap! {
            "hello" => 1,
            "world" => 2,
        };

        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn clear() {
        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            wg.clear();
        }

        assert!(table.read().unwrap().is_empty());
        assert!(table.write().unwrap().is_empty());
        assert!(table.read().unwrap().is_empty());
    }

    #[test]
    fn remove() {
        let expected = hashmap! {
            "hello" => 1,
        };

        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove("world"), Some(2));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn remove_entry() {
        let expected = hashmap! {
            "hello" => 1,
        };

        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove_entry("world"), Some(("world", 2)));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn shrink_to_fit_and_reserve() {
        let table = lockless::AsLockHandle::<&str, i32>::default();
        let initial_capacity;
        let additional = 10;
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            wg.shrink_to_fit();
            initial_capacity = wg.capacity();
            wg.reserve(additional);
            assert_ge!(wg.capacity(), initial_capacity + additional);
        }

        assert_ge!(
            table.read().unwrap().capacity(),
            initial_capacity + additional
        );
        assert_ge!(
            table.write().unwrap().capacity(),
            initial_capacity + additional
        );
        assert_ge!(
            table.read().unwrap().capacity(),
            initial_capacity + additional
        );
    }

    #[test]
    fn retain() {
        let expected = hashmap! {
            "joe" => -16,
            "world" => 0,
            "my" => 2
        };
        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 0);
            wg.insert("my", 2);
            wg.insert("name", -1);
            wg.insert("is", 123);
            wg.insert("joe", -16);
            wg.retain(|_, &mut v| v % 2 == 0);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn drain() {
        let expected = hashmap! {
            "hello" => 1,
            "world" => 1,
        };

        let table = lockless::AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 1);
            assert_eq!(*wg, expected);
            assert_eq!(
                wg.drain().collect::<std::collections::HashMap<_, _>>(),
                expected
            );
        }

        assert!(table.read().unwrap().is_empty());
        assert!(table.write().unwrap().is_empty());
        assert!(table.read().unwrap().is_empty());
    }

    #[test]
    fn debug_str() {
        let table = lockless::AsLockHandle::<i32, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert(12, -1);
        }

        assert_eq!(format!("{:?}", table), "AsLockHandle { writer: Writer { num_readers: 1, ops_to_replay: 1, standby_table: {} }, reader: Reader { num_readers: 1, active_table: {12: -1} } }");
        assert_eq!(
            format!("{:?}", table.write().unwrap()),
            "WriteGuard { swap_active_and_standby: true, num_readers: 1, ops_to_replay: 0, standby_table: {12: -1} }",
        );
        assert_eq!(
            format!("{:?}", table.read().unwrap()),
            "ReadGuard { active_table: {12: -1} }",
        );
    }
}

#[cfg(test)]
mod shared_test {
    use super::*;
    use maplit::*;
    use more_asserts::*;
    use std::sync::Arc;

    #[test]
    fn insert() {
        let expected = hashmap! {
            "hello" => 1,
            "world" => 2,
        };

        let table = Arc::new(shared::AsLock::<&str, i32>::default());
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn clear() {
        let table = shared::AsLock::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            wg.clear();
        }

        assert!(table.read().unwrap().is_empty());
        assert!(table.write().unwrap().is_empty());
        assert!(table.read().unwrap().is_empty());
    }

    #[test]
    fn remove() {
        let expected = hashmap! {
            "hello" => 1,
        };

        let table = shared::AsLock::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove("world"), Some(2));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn remove_entry() {
        let expected = hashmap! {
            "hello" => 1,
        };

        let table = shared::AsLock::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove_entry("world"), Some(("world", 2)));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn shrink_to_fit_and_reserve() {
        let table = shared::AsLock::<&str, i32>::default();
        let initial_capacity;
        let additional = 10;
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            wg.shrink_to_fit();
            initial_capacity = wg.capacity();
            wg.reserve(additional);
            assert_ge!(wg.capacity(), initial_capacity + additional);
        }

        assert_ge!(
            table.read().unwrap().capacity(),
            initial_capacity + additional
        );
        assert_ge!(
            table.write().unwrap().capacity(),
            initial_capacity + additional
        );
        assert_ge!(
            table.read().unwrap().capacity(),
            initial_capacity + additional
        );
    }

    #[test]
    fn retain() {
        let expected = hashmap! {
            "joe" => -16,
            "world" => 0,
            "my" => 2
        };
        let table = shared::AsLock::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 0);
            wg.insert("my", 2);
            wg.insert("name", -1);
            wg.insert("is", 123);
            wg.insert("joe", -16);
            wg.retain(|_, &mut v| v % 2 == 0);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn drain() {
        let expected = hashmap! {
            "hello" => 1,
            "world" => 1,
        };

        let table = shared::AsLock::<&str, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello", 1);
            wg.insert("world", 1);
            assert_eq!(*wg, expected);
            assert_eq!(
                wg.drain().collect::<std::collections::HashMap<_, _>>(),
                expected
            );
        }

        assert!(table.read().unwrap().is_empty());
        assert!(table.write().unwrap().is_empty());
        assert!(table.read().unwrap().is_empty());
    }

    #[test]
    fn debug_str() {
        let table = shared::AsLock::<i32, i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert(12, -1);
        }

        assert_eq!(
            format!("{:?}", table),
            "AsLock { num_ops_to_replay: 1, active_table: {12: -1} }",
        );
        assert_eq!(
            format!("{:?}", table.write().unwrap()),
            "WriteGuard { num_ops_to_replay: 0, standby_table: {12: -1} }",
        );
        assert_eq!(
            format!("{:?}", table.read().unwrap()),
            "ShardedLockReadGuard { lock: ShardedLock { data: {12: -1} } }",
        );
    }
}
