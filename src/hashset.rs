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

    pub struct Reader<T> {
        reader: primitives::Reader<HashSet<T>>,
    }

    impl<T> Reader<T> {
        pub fn read(&mut self) -> ReadGuard<'_, T> {
            ReadGuard {
                guard: self.reader.read(),
            }
        }
    }

    pub struct ReadGuard<'r, T> {
        guard: primitives::ReadGuard<'r, HashSet<T>>,
    }

    impl<'r, T> std::ops::Deref for ReadGuard<'r, T> {
        type Target = HashSet<T>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    pub struct Writer<T> {
        writer: primitives::SendWriter<HashSet<T>>,
    }

    impl<T> Writer<T>
    where
        T: Clone,
    {
        pub fn new() -> Writer<T> {
            Writer {
                writer: primitives::SendWriter::new(HashSet::new()),
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
        guard: primitives::SendWriteGuard<'w, HashSet<T>>,
    }

    impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
        type Target = HashSet<T>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

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

    struct Clear {}
    impl<'a, T> UpdateTables<'a, HashSet<T>, ()> for Clear {
        fn apply_first(&mut self, table: &'a mut HashSet<T>) {
            table.clear()
        }
        fn apply_second(mut self, table: &mut HashSet<T>) {
            self.apply_first(table);
        }
    }

    struct Remove<Q> {
        value_like: Q,
    }
    impl<'a, T, Q> UpdateTables<'a, HashSet<T>, bool> for Remove<Q>
    where
        Q: Eq + Hash,
        T: Eq + Hash + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &'a mut HashSet<T>) -> bool {
            table.remove(&self.value_like)
        }
        fn apply_second(mut self, table: &mut HashSet<T>) {
            self.apply_first(table);
        }
    }

    struct Take<Q> {
        value_like: Q,
    }
    impl<'a, T, Q> UpdateTables<'a, HashSet<T>, Option<T>> for Take<Q>
    where
        Q: Eq + Hash,
        T: Eq + Hash + std::borrow::Borrow<Q>,
    {
        fn apply_first(&mut self, table: &'a mut HashSet<T>) -> Option<T> {
            table.take(&self.value_like)
        }
        fn apply_second(mut self, table: &mut HashSet<T>) {
            self.apply_first(table);
        }
    }

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

    struct Drain {}
    impl<'a, T> UpdateTables<'a, HashSet<T>, std::collections::hash_set::Drain<'a, T>> for Drain
    where
        T: Eq + Hash,
    {
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

    impl<'w, 'a, T> WriteGuard<'w, T>
    where
        T: 'static + Eq + Hash + Clone + Send,
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
            Q: 'static + Hash + Eq + Send,
        {
            self.guard.update_tables(Remove { value_like })
        }

        pub fn take<Q>(&mut self, value_like: Q) -> Option<T>
        where
            T: std::borrow::Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard.update_tables(Take { value_like })
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard.update_tables(Reserve { additional })
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard.update_tables(ShrinkToFit {})
        }

        pub fn drain(&'a mut self) -> std::collections::hash_set::Drain<'a, T> {
            self.guard.update_tables(Drain {})
        }

        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Send + Clone + FnMut(&T) -> bool,
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
        let mut reader = writer.new_reader();
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
        let mut reader = writer.new_reader();
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
        let mut reader = writer.new_reader();
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
        let mut reader = writer.new_reader();
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
        let mut reader = writer.new_reader();
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
        let mut reader = writer.new_reader();
        {
            let mut wg = writer.write();
            wg.insert("hello");
            wg.insert("world");
            assert_eq!(*wg, expected);
            assert_eq!(
                wg.drain().collect::<std::collections::HashSet<_>>(),
                expected
            );
        }

        assert!(reader.read().is_empty());
        assert!(writer.write().is_empty());
        assert!(reader.read().is_empty());
    }
}
