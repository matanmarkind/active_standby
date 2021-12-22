/// Implementation of HashSet for use in the active_standby model.
/// hashset::AsLockHandle<T>, should function similarly to
/// Arc<RwLock<HashSet<T>>>.
use crate::primitives::UpdateTables;
use std::borrow::Borrow;
use std::collections::HashSet;
use std::hash::Hash;

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

struct Drain {}

impl<'a, T> UpdateTables<'a, HashSet<T>, std::collections::hash_set::Drain<'a, T>> for Drain {
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

pub mod lockless {
    use super::*;
    crate::generate_lockless_aslockhandle!(HashSet<T>);

    impl<'w, 'a, T> WriteGuard<'w, T>
    where
        T: 'static + Eq + Hash + Clone + Send,
    {
        pub fn clear(&mut self) {
            self.guard.update_tables_closure(move |table| table.clear())
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard
                .update_tables_closure(move |table| table.shrink_to_fit())
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard
                .update_tables_closure(move |table| table.reserve(additional))
        }

        pub fn insert(&mut self, value: T) -> bool {
            self.guard.update_tables(Insert { value })
        }

        pub fn replace(&mut self, value: T) -> Option<T> {
            self.guard.update_tables(Replace { value })
        }

        pub fn remove<Q>(&mut self, value_like: Q) -> bool
        where
            T: Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove(&value_like))
        }

        pub fn take<Q>(&mut self, value_like: Q) -> Option<T>
        where
            T: Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.take(&value_like))
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

        pub fn drain(&'a mut self) -> std::collections::hash_set::Drain<'a, T> {
            self.guard.update_tables(Drain {})
        }
    }
}

pub mod shared {
    use super::*;
    crate::generate_shared_aslock!(HashSet<T>);

    impl<'w, 'a, T> WriteGuard<'w, T>
    where
        T: 'static + Eq + Hash + Clone + Send,
    {
        pub fn clear(&mut self) {
            self.guard.update_tables_closure(move |table| table.clear())
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard
                .update_tables_closure(move |table| table.shrink_to_fit())
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard
                .update_tables_closure(move |table| table.reserve(additional))
        }

        pub fn insert(&mut self, value: T) -> bool {
            self.guard.update_tables(Insert { value })
        }

        pub fn replace(&mut self, value: T) -> Option<T> {
            self.guard.update_tables(Replace { value })
        }

        pub fn remove<Q>(&mut self, value_like: Q) -> bool
        where
            T: Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.remove(&value_like))
        }

        pub fn take<Q>(&mut self, value_like: Q) -> Option<T>
        where
            T: Borrow<Q>,
            Q: 'static + Hash + Eq + Send,
        {
            self.guard
                .update_tables_closure(move |table| table.take(&value_like))
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

        pub fn drain(&'a mut self) -> std::collections::hash_set::Drain<'a, T> {
            self.guard.update_tables(Drain {})
        }
    }
}

#[cfg(test)]
mod lockless_test {
    use super::*;
    use maplit::*;
    use more_asserts::*;

    #[test]
    fn insert_and_replace() {
        let expected = hashset! {
            "hello",
            "world",
        };

        let table = lockless::AsLockHandle::<&str>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            wg.replace("world");
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn clear() {
        let table = lockless::AsLockHandle::<&str>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            wg.clear();
        }

        assert!(table.read().unwrap().is_empty());
        assert!(table.write().unwrap().is_empty());
        assert!(table.read().unwrap().is_empty());
    }

    #[test]
    fn remove_and_take() {
        let expected = hashset! {
            "hello",
        };

        let table = lockless::AsLockHandle::<&str>::new(std::collections::HashSet::new());
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            wg.insert("I");
            assert_eq!(wg.remove("world"), true);
            assert_eq!(wg.take("I"), Some("I"));
            assert_eq!(wg.take("I"), None);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn shrink_to_fit_and_reserve() {
        let table = lockless::AsLockHandle::<&str>::from_identical(
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        );
        let initial_capacity;
        let additional = 10;
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
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
        let expected = hashset! {
            "joe",
            "world",
            "hello",
            "name",
        };
        let table = lockless::AsLockHandle::<&str>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            wg.insert("my");
            wg.insert("name");
            wg.insert("is");
            wg.insert("joe");
            wg.retain(|&k| k.len() > 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn drain() {
        let expected = hashset! {
            "hello" ,
            "world",
        };

        let table = lockless::AsLockHandle::<&str>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            assert_eq!(*wg, expected);
            assert_eq!(
                wg.drain().collect::<std::collections::HashSet<_>>(),
                expected
            );
        }

        assert!(table.read().unwrap().is_empty());
        assert!(table.write().unwrap().is_empty());
        assert!(table.read().unwrap().is_empty());
    }

    #[test]
    fn debug_str() {
        let table = lockless::AsLockHandle::<i32>::default();
        {
            table.write().unwrap().insert(12);
        }

        assert_eq!(format!("{:?}", table), "AsLockHandle { writer: Writer { num_readers: 1, ops_to_replay: 1, standby_table: {} }, reader: Reader { num_readers: 1, active_table: {12} } }",);
        assert_eq!(
            format!("{:?}", table.write().unwrap()),
            "WriteGuard { swap_active_and_standby: true, num_readers: 1, ops_to_replay: 0, standby_table: {12} }",
        );
        assert_eq!(
            format!("{:?}", table.read().unwrap()),
            "ReadGuard { active_table: {12} }",
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
    fn insert_and_replace() {
        let expected = hashset! {
            "hello",
            "world",
        };

        let table = Arc::new(shared::AsLock::<&str>::default());
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            wg.replace("world");
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn clear() {
        let table = shared::AsLock::<&str>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            wg.clear();
        }

        assert!(table.read().unwrap().is_empty());
        assert!(table.write().unwrap().is_empty());
        assert!(table.read().unwrap().is_empty());
    }

    #[test]
    fn remove_and_take() {
        let expected = hashset! {
            "hello",
        };

        let table = shared::AsLock::<&str>::new(std::collections::HashSet::new());
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            wg.insert("I");
            assert_eq!(wg.remove("world"), true);
            assert_eq!(wg.take("I"), Some("I"));
            assert_eq!(wg.take("I"), None);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn shrink_to_fit_and_reserve() {
        let table = shared::AsLock::<&str>::from_identical(
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        );
        let initial_capacity;
        let additional = 10;
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
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
        let expected = hashset! {
            "joe",
            "world",
            "hello",
            "name",
        };
        let table = shared::AsLock::<&str>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            wg.insert("my");
            wg.insert("name");
            wg.insert("is");
            wg.insert("joe");
            wg.retain(|&k| k.len() > 2);
            assert_eq!(*wg, expected);
        }

        assert_eq!(*table.read().unwrap(), expected);
        assert_eq!(*table.write().unwrap(), expected);
        assert_eq!(*table.read().unwrap(), expected);
    }

    #[test]
    fn drain() {
        let expected = hashset! {
            "hello" ,
            "world",
        };

        let table = shared::AsLock::<&str>::default();
        {
            let mut wg = table.write().unwrap();
            wg.insert("hello");
            wg.insert("world");
            assert_eq!(*wg, expected);
            assert_eq!(
                wg.drain().collect::<std::collections::HashSet<_>>(),
                expected
            );
        }

        assert!(table.read().unwrap().is_empty());
        assert!(table.write().unwrap().is_empty());
        assert!(table.read().unwrap().is_empty());
    }

    #[test]
    fn debug_str() {
        let table = shared::AsLock::<i32>::default();
        {
            table.write().unwrap().insert(12);
        }

        assert_eq!(format!("{:?}", table), "AsLock { num_ops_to_replay: 1 }",);
        assert_eq!(
            format!("{:?}", table.write().unwrap()),
            "WriteGuard { num_ops_to_replay: 0, standby_table: TableWriteGuard { standby_table: {12} } }",
        );
        assert_eq!(
            format!("{:?}", table.read().unwrap()),
            "ShardedLockReadGuard { lock: ShardedLock { data: {12} } }",
        );
    }
}
