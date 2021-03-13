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

    pub struct Reader<T> {
        reader: primitives::Reader<BTreeSet<T>>,
    }

    impl<T> Reader<T> {
        pub fn read(&self) -> ReadGuard<'_, T> {
            ReadGuard {
                guard: self.reader.read(),
            }
        }
    }

    pub struct ReadGuard<'r, T> {
        guard: primitives::ReadGuard<'r, BTreeSet<T>>,
    }

    impl<'r, T> std::ops::Deref for ReadGuard<'r, T> {
        type Target = BTreeSet<T>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    pub struct Writer<T> {
        writer: primitives::SendWriter<BTreeSet<T>>,
    }

    impl<T> Writer<T>
    where
        T: Clone + Ord,
    {
        pub fn new() -> Writer<T> {
            Writer {
                writer: primitives::SendWriter::new(BTreeSet::new()),
            }
        }
    }

    impl<T> Writer<T> {
        pub fn write(&mut self) -> WriteGuard<'_, T> {
            WriteGuard {
                guard: self.writer.write(),
            }
        }
        pub fn new_reader(&self) -> Reader<T> {
            Reader {
                reader: self.writer.new_reader(),
            }
        }
    }

    pub struct WriteGuard<'w, T> {
        guard: primitives::SendWriteGuard<'w, BTreeSet<T>>,
    }

    impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
        type Target = BTreeSet<T>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    struct Insert<T> {
        value: T,
    }
    impl<T> UpdateTables<BTreeSet<T>, bool> for Insert<T>
    where
        T: Ord + Clone,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<T>) -> bool {
            table.insert(self.value.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut BTreeSet<T>) {
            // Move the value instead of cloning.
            table.insert(self.value);
        }
    }

    struct Replace<T> {
        value: T,
    }
    impl<T> UpdateTables<BTreeSet<T>, Option<T>> for Replace<T>
    where
        T: Ord + Clone,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<T>) -> Option<T> {
            table.replace(self.value.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut BTreeSet<T>) {
            // Move the value instead of cloning.
            table.replace(self.value);
        }
    }

    struct Clear {}
    impl<T> UpdateTables<BTreeSet<T>, ()> for Clear
    where
        T: Ord + Clone,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<T>) {
            table.clear()
        }
    }

    struct Remove<Q> {
        value_like: Q,
    }
    impl<T, Q> UpdateTables<BTreeSet<T>, bool> for Remove<Q>
    where
        Q: Ord,
        T: Ord + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<T>) -> bool {
            table.remove(&self.value_like)
        }
    }

    struct Take<Q> {
        value_like: Q,
    }
    impl<T, Q> UpdateTables<BTreeSet<T>, Option<T>> for Take<Q>
    where
        Q: Ord,
        T: Ord + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<T>) -> Option<T> {
            table.take(&self.value_like)
        }
    }

    struct Append<T> {
        other: BTreeSet<T>,
    }
    impl<T> UpdateTables<BTreeSet<T>, ()> for Append<T>
    where
        T: Ord + Clone,
    {
        fn apply_first(&mut self, table: &mut BTreeSet<T>) {
            for k in self.other.iter() {
                table.insert(k.clone());
            }
        }
        fn apply_second(mut self: Box<Self>, table: &mut BTreeSet<T>) {
            table.append(&mut self.other)
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
