/// Implementation of HashSet for use in the active_standby model.
///
/// Specifically this allows users to call mutating functions on the
/// hashset::WriteGuard like they would on a HashSet. Functions that return a
/// reference to data owned by the underlying Vec will have different return
/// values because we don't allow tying return values to the underlying data to
/// avoid ever returning a mutable reference which the reader will use to change
/// the table without recording it.

pub mod hashset {
    use crate::primitives;
    use crate::primitives::UpdateTables;
    use std::collections::HashSet;
    use std::hash::Hash;

    pub struct Reader<K> {
        reader: primitives::Reader<HashSet<K>>,
    }

    impl<K> Reader<K> {
        pub fn read(&self) -> ReadGuard<'_, K> {
            ReadGuard {
                guard: self.reader.read(),
            }
        }
    }

    pub struct ReadGuard<'r, K> {
        guard: primitives::ReadGuard<'r, HashSet<K>>,
    }

    impl<'r, K> std::ops::Deref for ReadGuard<'r, K> {
        type Target = HashSet<K>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    pub struct Writer<K> {
        writer: primitives::SendWriter<HashSet<K>>,
    }

    impl<K> Writer<K>
    where
        K: Clone,
    {
        pub fn new() -> Writer<K> {
            Writer {
                writer: primitives::SendWriter::new(HashSet::new()),
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
        guard: primitives::SendWriteGuard<'w, HashSet<K>>,
    }

    impl<'w, K> std::ops::Deref for WriteGuard<'w, K> {
        type Target = HashSet<K>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    struct Insert<K> {
        key: K,
    }
    impl<K> UpdateTables<HashSet<K>, bool> for Insert<K>
    where
        K: Eq + Hash + Clone,
    {
        fn apply_first(&mut self, table: &mut HashSet<K>) -> bool {
            table.insert(self.key.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut HashSet<K>) {
            // Move the value instead of cloning.
            table.insert(self.key);
        }
    }

    struct Replace<K> {
        key: K,
    }
    impl<K> UpdateTables<HashSet<K>, Option<K>> for Replace<K>
    where
        K: Eq + Hash + Clone,
    {
        fn apply_first(&mut self, table: &mut HashSet<K>) -> Option<K> {
            table.replace(self.key.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut HashSet<K>) {
            // Move the value instead of cloning.
            table.replace(self.key);
        }
    }

    struct Clear {}
    impl<K> UpdateTables<HashSet<K>, ()> for Clear {
        fn apply_first(&mut self, table: &mut HashSet<K>) {
            table.clear()
        }
    }

    struct Remove<Q> {
        key_like: Q,
    }
    impl<K, Q> UpdateTables<HashSet<K>, bool> for Remove<Q>
    where
        Q: Eq + Hash,
        K: Eq + Hash + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &mut HashSet<K>) -> bool {
            table.remove(&self.key_like)
        }
    }

    struct Take<Q> {
        key_like: Q,
    }
    impl<K, Q> UpdateTables<HashSet<K>, Option<K>> for Take<Q>
    where
        Q: Eq + Hash,
        K: Eq + Hash + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &mut HashSet<K>) -> Option<K> {
            table.take(&self.key_like)
        }
    }

    struct Reserve {
        additional: usize,
    }
    impl<K> UpdateTables<HashSet<K>, ()> for Reserve
    where
        K: Eq + Hash,
    {
        fn apply_first(&mut self, table: &mut HashSet<K>) {
            table.reserve(self.additional)
        }
    }

    struct ShrinkToFit {}
    impl<K> UpdateTables<HashSet<K>, ()> for ShrinkToFit
    where
        K: Eq + Hash,
    {
        fn apply_first(&mut self, table: &mut HashSet<K>) {
            table.shrink_to_fit()
        }
    }

    struct Drain {}
    impl<K> UpdateTables<HashSet<K>, HashSet<K>> for Drain
    where
        K: Eq + Hash,
    {
        fn apply_first(&mut self, table: &mut HashSet<K>) -> HashSet<K> {
            table.drain().collect()
        }
    }

    struct Retain<K, F>
    where
        F: 'static + Clone + FnMut(&K) -> bool,
    {
        f: F,
        _compile_k_v: std::marker::PhantomData<K>,
    }
    impl<K, F> UpdateTables<HashSet<K>, ()> for Retain<K, F>
    where
        K: Eq + Hash,
        F: 'static + Clone + FnMut(&K) -> bool,
    {
        fn apply_first(&mut self, table: &mut HashSet<K>) {
            table.retain(self.f.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut HashSet<K>) {
            table.retain(self.f)
        }
    }

    impl<'w, K> WriteGuard<'w, K>
    where
        K: 'static + Eq + Hash + Clone + Send,
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
            Q: 'static + Hash + Eq + Send,
        {
            self.guard.update_tables(Remove { key_like })
        }

        pub fn take<Q>(&mut self, key_like: Q) -> Option<K>
        where
            K: std::borrow::Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard.update_tables(Take { key_like })
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard.update_tables(Reserve { additional })
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard.update_tables(ShrinkToFit {})
        }

        pub fn drain(&mut self) -> HashSet<K> {
            self.guard.update_tables(Drain {})
        }

        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Send + Clone + FnMut(&K) -> bool,
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
    use super::hashset::*;
    use maplit::*;
    use more_asserts::*;

    #[test]
    fn insert_and_replace() {
        let expected = hashset! {
            "hello",
            "world",
        };

        let mut writer = Writer::<&str>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello");
            wg.insert("world");
            wg.replace("world");
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
        let expected = hashset! {
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
    fn shrink_to_fit_and_reserve() {
        let mut writer = Writer::<&str>::new();
        let reader = writer.new_reader();
        let initial_capacity;
        let additional = 10;
        {
            let mut wg = writer.write();
            wg.insert("hello");
            wg.insert("world");
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
        let expected = hashset! {
            "joe",
            "world",
            "hello",
            "name",
        };
        let mut writer = Writer::<&str>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello");
            wg.insert("world");
            wg.insert("my");
            wg.insert("name");
            wg.insert("is");
            wg.insert("joe");
            wg.retain(|&k| k.len() > 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*reader.read(), expected);
        assert_eq!(*writer.write(), expected);
        assert_eq!(*reader.read(), expected);
    }

    #[test]
    fn drain() {
        let expected = hashset! {
            "hello" ,
            "world",
        };

        let mut writer = Writer::<&str>::new();
        let reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello");
            wg.insert("world");
            assert_eq!(*wg, expected);
            assert_eq!(wg.drain(), expected);
        }

        assert!(reader.read().is_empty());
        assert!(writer.write().is_empty());
        assert!(reader.read().is_empty());
    }
}
