/// Implementation of BTreeSet for use in the active_standby model.
///
/// Specifically this allows users to call mutating functions on the
/// btreeset::WriteGuard like they would on a BTreeSet. Functions that return a
/// reference to data owned by the underlying Vec will have different return
/// values because we don't allow tying return values to the underlying data to
/// avoid ever returning a mutable reference which the reader will use to change
/// the table without recording it.

pub mod btreeset {
    use crate::primitives;
    use crate::primitives::UpdateTables;
    use std::collections::BTreeSet;

    pub struct Reader<K> {
        reader: primitives::Reader<BTreeSet<K>>,
    }

    impl<K> Reader<K> {
        pub fn read(&self) -> ReadGuard<'_, K> {
            ReadGuard {
                guard: self.reader.read(),
            }
        }
    }

    pub struct ReadGuard<'r, K> {
        guard: primitives::ReadGuard<'r, BTreeSet<K>>,
    }

    impl<'r, K> std::ops::Deref for ReadGuard<'r, K> {
        type Target = BTreeSet<K>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    pub struct Writer<K> {
        writer: primitives::SendWriter<BTreeSet<K>>,
    }

    impl<K> Writer<K>
    where
        K: Clone + Ord,
    {
        pub fn new() -> Writer<K> {
            Writer {
                writer: primitives::SendWriter::new(BTreeSet::new()),
            }
        }
    }

    impl<K> Writer<K> {
        pub fn write(&mut self) -> WriteGuard<'_, K> {
            WriteGuard {
                guard: self.writer.write(),
            }
        }
        pub fn new_reader(&self) -> Reader<K> {
            Reader {
                reader: self.writer.new_reader(),
            }
        }
    }

    pub struct WriteGuard<'w, K> {
        guard: primitives::SendWriteGuard<'w, BTreeSet<K>>,
    }

    impl<'w, K> std::ops::Deref for WriteGuard<'w, K> {
        type Target = BTreeSet<K>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    struct Insert<K> {
        key: K,
    }
    impl<K> UpdateTables<BTreeSet<K>, bool> for Insert<K>
    where
        K: Ord + Clone,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<K>) -> bool {
            table.insert(self.key.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut BTreeSet<K>) {
            // Move the value instead of cloning.
            table.insert(self.key);
        }
    }

    struct Replace<K> {
        key: K,
    }
    impl<K> UpdateTables<BTreeSet<K>, Option<K>> for Replace<K>
    where
        K: Ord + Clone,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<K>) -> Option<K> {
            table.replace(self.key.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut BTreeSet<K>) {
            // Move the value instead of cloning.
            table.replace(self.key);
        }
    }

    struct Clear {}
    impl<K> UpdateTables<BTreeSet<K>, ()> for Clear
    where
        K: Ord + Clone,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<K>) {
            table.clear()
        }
    }

    struct Remove<Q> {
        key_like: Q,
    }
    impl<K, Q> UpdateTables<BTreeSet<K>, bool> for Remove<Q>
    where
        Q: Ord,
        K: Ord + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<K>) -> bool {
            table.remove(&self.key_like)
        }
    }

    struct Take<Q> {
        key_like: Q,
    }
    impl<K, Q> UpdateTables<BTreeSet<K>, Option<K>> for Take<Q>
    where
        Q: Ord,
        K: Ord + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<K>) -> Option<K> {
            table.take(&self.key_like)
        }
    }

    struct Append<K> {
        other: BTreeSet<K>,
    }
    impl<K> UpdateTables<BTreeSet<K>, ()> for Append<K>
    where
        K: Ord + Clone,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<K>) {
            for k in self.other.iter() {
                table.insert(k.clone());
            }
        }
        fn apply_second(mut self: Box<Self>, table: &mut BTreeSet<K>) {
            table.append(&mut self.other)
        }
    }

    impl<'w, K> WriteGuard<'w, K>
    where
        K: 'static + Ord + Clone + Send,
    {
        pub fn insert(&mut self, key: K) -> bool {
            self.guard.update_tables(Insert { key })
        }

        pub fn replace(&mut self, key: K) -> Option<K> {
            self.guard.update_tables(Replace { key })
        }

        pub fn clear(&mut self) {
            self.guard.update_tables(Clear {})
        }

        pub fn remove<Q>(&mut self, key_like: Q) -> bool
        where
            K: std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard.update_tables(Remove { key_like })
        }

        pub fn take<Q>(&mut self, key_like: Q) -> Option<K>
        where
            K: std::borrow::Borrow<Q>,
            Q: 'static + Ord + Send,
        {
            self.guard.update_tables(Take { key_like })
        }

        pub fn append(&mut self, other: BTreeSet<K>) {
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

        let mut writer = Writer::<&str>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello");
            wg.insert("world");
            assert_eq!(*wg, expected);
        }

        assert_eq!(*reader.read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*reader.read(), expected);
    }

    #[test]
    fn clear() {
        let mut writer = Writer::<&str>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello");
            wg.insert("world");
            wg.clear();
        }

        assert!(reader.read().is_empty());
        assert!((*writer.write()).is_empty());
        assert!(reader.read().is_empty());
    }

    #[test]
    fn remove_and_take() {
        let expected = btreeset! {
            "hello",
        };
        let mut writer = Writer::<&str>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello");
            wg.insert("world");
            wg.insert("I");
            assert_eq!(wg.remove("world"), true);
            assert_eq!(wg.take("I"), Some("I"));
            assert_eq!(wg.take("I"), None);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*reader.read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*reader.read(), expected);
    }

    #[test]
    fn append() {
        let expected = btreeset! {
            "hello",
            "world",
            "name's",
            "joe",
        };

        let mut writer = Writer::<&str>::new();
        let reader = writer.new_reader();
        {
            let map1 = btreeset! {
                "hello",
                "world",
            };
            let map2 = btreeset! {
                "name's" ,
                "joe" ,
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
