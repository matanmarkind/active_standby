/// Implementation of HashMap for use in the active_standby model.
/// hashsmap::AsLockHandle<K, V>, should function similarly to
/// Arc<RwLock<HashMap<K, V>>>.
use crate::primitives::UpdateTables;
use std::collections::HashMap;
use std::hash::Hash;

crate::generate_aslock_handle!(HashMap<K, V>);

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

struct Clear {}

impl<'a, K, V> UpdateTables<'a, HashMap<K, V>, ()> for Clear {
    fn apply_first(&mut self, table: &'a mut HashMap<K, V>) {
        table.clear()
    }
    fn apply_second(mut self, table: &mut HashMap<K, V>) {
        self.apply_first(table);
    }
}

struct Remove<Q> {
    key_like: Q,
}

impl<'a, K, V, Q> UpdateTables<'a, HashMap<K, V>, Option<V>> for Remove<Q>
where
    Q: Eq + Hash,
    K: Eq + Hash + std::borrow::Borrow<Q>,
{
    fn apply_first(&mut self, table: &'a mut HashMap<K, V>) -> Option<V> {
        table.remove(&self.key_like)
    }
    fn apply_second(mut self, table: &mut HashMap<K, V>) {
        self.apply_first(table);
    }
}

struct RemoveEntry<Q> {
    key_like: Q,
}

impl<'a, K, V, Q> UpdateTables<'a, HashMap<K, V>, Option<(K, V)>> for RemoveEntry<Q>
where
    Q: Eq + Hash,
    K: Eq + Hash + std::borrow::Borrow<Q>,
{
    fn apply_first(&mut self, table: &'a mut HashMap<K, V>) -> Option<(K, V)> {
        table.remove_entry(&self.key_like)
    }
    fn apply_second(mut self, table: &mut HashMap<K, V>) {
        self.apply_first(table);
    }
}

struct Reserve {
    additional: usize,
}

impl<'a, K, V> UpdateTables<'a, HashMap<K, V>, ()> for Reserve
where
    K: Eq + Hash,
{
    fn apply_first(&mut self, table: &'a mut HashMap<K, V>) {
        table.reserve(self.additional)
    }
    fn apply_second(mut self, table: &mut HashMap<K, V>) {
        self.apply_first(table);
    }
}

struct ShrinkToFit {}

impl<'a, K, V> UpdateTables<'a, HashMap<K, V>, ()> for ShrinkToFit
where
    K: Eq + Hash,
{
    fn apply_first(&mut self, table: &'a mut HashMap<K, V>) {
        table.shrink_to_fit()
    }
    fn apply_second(mut self, table: &mut HashMap<K, V>) {
        self.apply_first(table);
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

impl<'w, 'a, K, V> WriteGuard<'w, K, V>
where
    K: 'static + Eq + Hash + Clone + Send,
    V: 'static + Clone + Send,
{
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.guard.update_tables(Insert { key, value })
    }

    pub fn clear(&mut self) {
        self.guard.update_tables(Clear {})
    }

    pub fn remove<Q>(&mut self, key_like: Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: 'static + Hash + Eq + Send,
    {
        self.guard.update_tables(Remove { key_like })
    }

    pub fn remove_entry<Q>(&mut self, key_like: Q) -> Option<(K, V)>
    where
        K: std::borrow::Borrow<Q>,
        Q: 'static + Hash + Eq + Send,
    {
        self.guard.update_tables(RemoveEntry { key_like })
    }

    pub fn reserve(&mut self, additional: usize) {
        self.guard.update_tables(Reserve { additional })
    }

    pub fn shrink_to_fit(&mut self) {
        self.guard.update_tables(ShrinkToFit {})
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

#[cfg(test)]
mod test {
    use super::hashmap::*;
    use maplit::*;
    use more_asserts::*;

    #[test]
    fn insert() {
        let expected = hashmap! {
            "hello" => 1,
            "world" => 2,
        };

        let table = AsLockHandle::<&str, i32>::default();
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
        let table = AsLockHandle::<&str, i32>::default();
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
        let expected = hashmap! {
            "hello" => 1,
        };

        let table = AsLockHandle::<&str, i32>::default();
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
        let expected = hashmap! {
            "hello" => 1,
        };

        let table = AsLockHandle::<&str, i32>::default();
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
    fn shrink_to_fit_and_reserve() {
        let table = AsLockHandle::<&str, i32>::default();
        let initial_capacity;
        let additional = 10;
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
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
        let expected = hashmap! {
            "joe" => -16,
            "world" => 0,
            "my" => 2
        };
        let table = AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 0);
            wg.insert("my", 2);
            wg.insert("name", -1);
            wg.insert("is", 123);
            wg.insert("joe", -16);
            wg.retain(|_, &mut v| v % 2 == 0);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read(), expected);
        assert_eq!(*table.write(), expected);
        assert_eq!(*table.read(), expected);
    }

    #[test]
    fn drain() {
        let expected = hashmap! {
            "hello" => 1,
            "world" => 1,
        };

        let table = AsLockHandle::<&str, i32>::default();
        {
            let mut wg = table.write();
            wg.insert("hello", 1);
            wg.insert("world", 1);
            assert_eq!(*wg, expected);
            assert_eq!(
                wg.drain().collect::<std::collections::HashMap<_, _>>(),
                expected
            );
        }

        assert!(table.read().is_empty());
        assert!(table.write().is_empty());
        assert!(table.read().is_empty());
    }

    #[test]
    fn debug_str() {
        let table = AsLockHandle::<i32, i32>::default();
        {
            let mut wg = table.write();
            wg.insert(12, -1);
        }

        assert_eq!(
            format!("{:?}", table),
            "AsLockHandle { reader: ReadGuard { active_table: {12: -1} } }",
        );
    }
}
