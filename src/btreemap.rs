/// Implementation of BTreeMap for use in the active_standby model.
///
/// Specifically this allows users to call mutating functions on the
/// btreemap::WriteGuard like they would on a BTreeMap. Functions that return a
/// reference to data owned by the underlying Vec will have different return
/// values because we don't allow tying return values to the underlying data to
/// avoid ever returning a mutable reference which the reader will use to change
/// the table without recording it.

pub mod btreemap {
    use crate::primitives;
    use crate::primitives::UpdateTables;
    use std::collections::BTreeMap;

    pub struct Reader<K, V> {
        reader: primitives::Reader<BTreeMap<K, V>>,
    }

    impl<K, V> Reader<K, V> {
        pub fn read(&mut self) -> ReadGuard<'_, K, V> {
            ReadGuard {
                guard: self.reader.read(),
            }
        }
    }

    pub struct ReadGuard<'r, K, V> {
        guard: primitives::ReadGuard<'r, BTreeMap<K, V>>,
    }

    impl<'r, K, V> std::ops::Deref for ReadGuard<'r, K, V> {
        type Target = BTreeMap<K, V>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    pub struct Writer<K, V> {
        writer: primitives::SendWriter<BTreeMap<K, V>>,
    }

    impl<K, V> Writer<K, V>
    where
        K: Clone + Ord,
        V: Clone,
    {
        pub fn new() -> Writer<K, V> {
            Writer {
                writer: primitives::SendWriter::new(BTreeMap::new()),
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
        guard: primitives::SendWriteGuard<'w, BTreeMap<K, V>>,
    }

    impl<'w, K, V> std::ops::Deref for WriteGuard<'w, K, V> {
        type Target = BTreeMap<K, V>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

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

    struct Clear {}
    impl<'a, K, V> UpdateTables<'a, BTreeMap<K, V>, ()> for Clear
    where
        K: Ord,
    {
        fn apply_first(&mut self, table: &'a mut BTreeMap<K, V>) {
            table.clear()
        }
        fn apply_second(self, table: &mut BTreeMap<K, V>) {
            table.clear()
        }
    }

    struct Remove<Q> {
        key_like: Q,
    }
    impl<'a, K, V, Q> UpdateTables<'a, BTreeMap<K, V>, Option<V>> for Remove<Q>
    where
        Q: Ord,
        K: Ord + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &'a mut BTreeMap<K, V>) -> Option<V> {
            table.remove(&self.key_like)
        }
        fn apply_second(self, table: &mut BTreeMap<K, V>) {
            table.remove(&self.key_like);
        }
    }

    struct RemoveEntry<Q> {
        key_like: Q,
    }
    impl<'a, K, V, Q> UpdateTables<'a, BTreeMap<K, V>, Option<(K, V)>> for RemoveEntry<Q>
    where
        Q: Ord,
        K: Ord + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &'a mut BTreeMap<K, V>) -> Option<(K, V)> {
            table.remove_entry(&self.key_like)
        }
        fn apply_second(self, table: &mut BTreeMap<K, V>) {
            table.remove_entry(&self.key_like);
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

    impl<'w, K, V> WriteGuard<'w, K, V>
    where
        K: 'static + Ord + Clone + Send,
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
            K: Ord + std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard.update_tables(Remove { key_like })
        }

        pub fn remove_entry<Q>(&mut self, key_like: Q) -> Option<(K, V)>
        where
            K: std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard.update_tables(RemoveEntry { key_like })
        }

        pub fn append(&mut self, other: BTreeMap<K, V>) {
            self.guard.update_tables(Append { other })
        }
    }
}

#[cfg(test)]
mod test {
    use super::btreemap::*;
    use maplit::*;

    #[test]
    fn insert() {
        let expected = btreemap! {
            "hello" => 1,
            "world" => 2,
        };

        let mut writer = Writer::<&str, i32>::new();
        let mut reader = writer.new_reader();
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
        let mut reader = writer.new_reader();
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
        let expected = btreemap! {
            "hello" => 1,
        };

        let mut writer = Writer::<&str, i32>::new();
        let mut reader = writer.new_reader();
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
        let expected = btreemap! {
            "hello" => 1,
        };

        let mut writer = Writer::<&str, i32>::new();
        let mut reader = writer.new_reader();
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
    fn append() {
        let expected = btreemap! {
            "hello" => 1,
            "world" => 2,
            "name's" => 3,
            "joe" => 4,
        };

        let mut writer = Writer::<&str, i32>::new();
        let mut reader = writer.new_reader();
        {
            let map1 = btreemap! {
                "hello" => 1,
                "world" => 2,
            };
            let map2 = btreemap! {
                "name's" => 3,
                "joe" => 4,
            };
            let mut wg = writer.write();
            wg.append(map1);
            wg.append(map2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*reader.read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*reader.read(), expected);
    }
}
