/// Implementation of HashMap for use in the active_standby model.
///
/// Specifically this allows users to call mutating functions on the
/// hashmap::WriteGuard like they would on a Vec. Functions that return a reference
/// to data owned by the underlying Vec will have different return values
/// because we don't allow tying return values to the underlying data to avoid
/// ever returning a mutable reference which the reader will use to change the
/// table without recording it.

pub mod hashmap {
    use crate::primitives;
    use crate::primitives::UpdateTables;
    use std::collections::HashMap;
    use std::hash::Hash;

    pub struct Reader<K, V> {
        reader: primitives::Reader<HashMap<K, V>>,
    }

    impl<K, V> Reader<K, V> {
        pub fn read(&self) -> ReadGuard<'_, K, V> {
            ReadGuard {
                guard: self.reader.read(),
            }
        }
    }

    pub struct ReadGuard<'r, K, V> {
        guard: primitives::ReadGuard<'r, HashMap<K, V>>,
    }

    impl<'r, K, V> std::ops::Deref for ReadGuard<'r, K, V> {
        type Target = HashMap<K, V>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    pub struct Writer<K, V> {
        writer: primitives::SendWriter<HashMap<K, V>>,
    }

    impl<K, V> Writer<K, V>
    where
        K: Clone,
        V: Clone,
    {
        pub fn new() -> Writer<K, V> {
            Writer {
                writer: primitives::SendWriter::new(HashMap::new()),
            }
        }
    }

    impl<K, V> Writer<K, V> {
        pub fn write(&mut self) -> WriteGuard<'_, K, V> {
            WriteGuard {
                guard: self.writer.write(),
            }
        }
        pub fn new_reader(&self) -> Reader<K, V> {
            Reader {
                reader: self.writer.new_reader(),
            }
        }
    }

    pub struct WriteGuard<'w, K, V> {
        guard: primitives::SendWriteGuard<'w, HashMap<K, V>>,
    }

    impl<'w, K, V> std::ops::Deref for WriteGuard<'w, K, V> {
        type Target = HashMap<K, V>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    struct Insert<K, V> {
        key: K,
        value: V,
    }
    impl<K, V> UpdateTables<HashMap<K, V>, Option<V>> for Insert<K, V>
    where
        K: Eq + Hash + Clone,
        V: Clone,
    {
        fn apply_first(&mut self, table: &mut HashMap<K, V>) -> Option<V> {
            table.insert(self.key.clone(), self.value.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut HashMap<K, V>) {
            // Move the value instead of cloning.
            table.insert(self.key, self.value);
        }
    }

    struct Clear {}
    impl<K, V> UpdateTables<HashMap<K, V>, ()> for Clear {
        fn apply_first(&mut self, table: &mut HashMap<K, V>) {
            table.clear()
        }
    }

    struct Remove<Q> {
        key_like: Q,
    }
    impl<K, V, Q> UpdateTables<HashMap<K, V>, Option<V>> for Remove<Q>
    where
        Q: Eq + Hash,
        K: Eq + Hash + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &mut HashMap<K, V>) -> Option<V> {
            table.remove(&self.key_like)
        }
    }

    struct RemoveEntry<Q> {
        key_like: Q,
    }
    impl<K, V, Q> UpdateTables<HashMap<K, V>, Option<(K, V)>> for RemoveEntry<Q>
    where
        Q: Eq + Hash,
        K: Eq + Hash + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &mut HashMap<K, V>) -> Option<(K, V)> {
            table.remove_entry(&self.key_like)
        }
    }

    struct Reserve {
        additional: usize,
    }
    impl<K, V> UpdateTables<HashMap<K, V>, ()> for Reserve
    where
        K: Eq + Hash,
    {
        fn apply_first(&mut self, table: &mut HashMap<K, V>) {
            table.reserve(self.additional)
        }
    }

    struct ShrinkToFit {}
    impl<K, V> UpdateTables<HashMap<K, V>, ()> for ShrinkToFit
    where
        K: Eq + Hash,
    {
        fn apply_first(&mut self, table: &mut HashMap<K, V>) {
            table.shrink_to_fit()
        }
    }

    struct Drain {}
    impl<K, V> UpdateTables<HashMap<K, V>, HashMap<K, V>> for Drain
    where
        K: Eq + Hash,
    {
        fn apply_first(&mut self, table: &mut HashMap<K, V>) -> HashMap<K, V> {
            table.drain().collect()
        }
    }

    struct Retain<K, V, F>
    where
        F: 'static + Clone + FnMut(&K, &mut V) -> bool,
    {
        f: F,
        _compile_k_v: std::marker::PhantomData<(K, V)>,
    }
    impl<K, V, F> UpdateTables<HashMap<K, V>, ()> for Retain<K, V, F>
    where
        K: Eq + Hash,
        F: 'static + Clone + FnMut(&K, &mut V) -> bool,
    {
        fn apply_first(&mut self, table: &mut HashMap<K, V>) {
            table.retain(self.f.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut HashMap<K, V>) {
            table.retain(self.f)
        }
    }

    impl<'w, K, V> WriteGuard<'w, K, V>
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

        pub fn drain(&mut self) -> HashMap<K, V> {
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

        let mut writer = Writer::<&str, i32>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*reader.read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*reader.read(), expected);
    }

    #[test]
    fn clear() {
        let mut writer = Writer::<&str, i32>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            wg.clear();
        }

        assert!(reader.read().is_empty());
        // assert_eq!(*writer.write(), expected);
        // assert_eq!(*reader.read(), expected);
    }

    #[test]
    fn remove() {
        let expected = hashmap! {
            "hello" => 1,
        };

        let mut writer = Writer::<&str, i32>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove("world"), Some(2));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*reader.read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*reader.read(), expected);
    }

    #[test]
    fn remove_entry() {
        let expected = hashmap! {
            "hello" => 1,
        };

        let mut writer = Writer::<&str, i32>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            assert_eq!(wg.remove_entry("world"), Some(("world", 2)));
            assert_eq!(*wg, expected);
        }

        assert_eq!(*reader.read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*reader.read(), expected);
    }

    #[test]
    fn shrink_to_fit_and_reserve() {
        let mut writer = Writer::<&str, i32>::new();
        let reader = writer.new_reader();
        let initial_capacity;
        let additional = 10;
        {
            let mut wg = writer.write();
            wg.insert("hello", 1);
            wg.insert("world", 2);
            wg.shrink_to_fit();
            initial_capacity = wg.capacity();
            wg.reserve(additional);
            assert_ge!(wg.capacity(), initial_capacity + additional);
        }

        assert_ge!(reader.read().capacity(), initial_capacity + additional);
        assert_ge!(writer.write().capacity(), initial_capacity + additional);
        assert_ge!(reader.read().capacity(), initial_capacity + additional);
    }

    #[test]
    fn retain() {
        let expected = hashmap! {
            "joe" => -16,
            "world" => 0,
            "my" => 2
        };
        let mut writer = Writer::<&str, i32>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello", 1);
            wg.insert("world", 0);
            wg.insert("my", 2);
            wg.insert("name", -1);
            wg.insert("is", 123);
            wg.insert("joe", -16);
            wg.retain(|_, &mut v| v % 2 == 0);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*reader.read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*reader.read(), expected);
    }

    #[test]
    fn drain() {
        let expected = hashmap! {
            "hello" => 1,
            "world" => 1,
        };

        let mut writer = Writer::<&str, i32>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello", 1);
            wg.insert("world", 1);
            assert_eq!(*wg, expected);
            assert_eq!(wg.drain(), expected);
        }

        assert!(reader.read().is_empty());
        assert!(writer.write().is_empty());
        assert!(reader.read().is_empty());
    }
}
