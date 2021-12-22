/// Implementation of Vec for use in the active_standby model.
/// vec::lockless::AsLockHandle<T>, should function similarly to
/// Arc<RwLock<Vec<T>>>.
use crate::primitives::UpdateTables;
use std::ops::RangeBounds;

// Define the functions that the active_standby vector will have. Note that we
// only do this once, since both lockless & shared use the same UpdateTables
// trait.

struct Push<T> {
    value: T,
}

impl<'a, T> UpdateTables<'a, Vec<T>, ()> for Push<T>
where
    T: Clone,
{
    fn apply_first(&mut self, table: &'a mut Vec<T>) {
        table.push(self.value.clone());
    }
    fn apply_second(self, table: &mut Vec<T>) {
        table.push(self.value); // Move the value instead of cloning.
    }
}

struct Insert<T> {
    index: usize,
    element: T,
}

impl<'a, T> UpdateTables<'a, Vec<T>, ()> for Insert<T>
where
    T: Clone,
{
    fn apply_first(&mut self, table: &'a mut Vec<T>) {
        table.insert(self.index, self.element.clone())
    }
    fn apply_second(self, table: &mut Vec<T>) {
        // Move the value instead of cloning.
        table.insert(self.index, self.element)
    }
}

struct Drain<R>
where
    R: 'static + Clone + RangeBounds<usize>,
{
    range: R,
}

impl<'a, T, R> UpdateTables<'a, Vec<T>, std::vec::Drain<'a, T>> for Drain<R>
where
    R: 'static + Clone + RangeBounds<usize>,
{
    fn apply_first(&mut self, table: &'a mut Vec<T>) -> std::vec::Drain<'a, T> {
        table.drain(self.range.clone())
    }
    fn apply_second(self, table: &mut Vec<T>) {
        table.drain(self.range);
    }
}

struct Retain<T, F>
where
    F: 'static + Clone + FnMut(&T) -> bool,
{
    f: F,
    _compile_t: std::marker::PhantomData<fn(*const T)>,
}

impl<'a, T, F> UpdateTables<'a, Vec<T>, ()> for Retain<T, F>
where
    F: 'static + Clone + FnMut(&T) -> bool,
{
    fn apply_first(&mut self, table: &'a mut Vec<T>) {
        table.retain(self.f.clone())
    }
    fn apply_second(self, table: &mut Vec<T>) {
        table.retain(self.f)
    }
}

pub mod lockless {
    use super::*;
    crate::generate_lockless_aslockhandle!(Vec<T>);

    impl<'w, T> WriteGuard<'w, T>
    where
        T: 'static + Clone + Send,
    {
        pub fn push(&mut self, value: T) {
            self.guard.update_tables(Push { value })
        }

        pub fn insert(&mut self, index: usize, element: T) {
            self.guard.update_tables(Insert { index, element })
        }
    }

    impl<'w, T> WriteGuard<'w, T> {
        pub fn clear(&mut self) {
            self.guard.update_tables_closure(move |table| table.clear())
        }
        pub fn pop(&mut self) -> Option<T> {
            self.guard.update_tables_closure(move |table| table.pop())
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard
                .update_tables_closure(move |table| table.reserve(additional))
        }

        pub fn reserve_exact(&mut self, additional: usize) {
            self.guard
                .update_tables_closure(move |table| table.reserve_exact(additional))
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard
                .update_tables_closure(move |table| table.shrink_to_fit())
        }

        pub fn truncate(&mut self, len: usize) {
            self.guard
                .update_tables_closure(move |table| table.truncate(len))
        }

        pub fn swap_remove(&mut self, index: usize) -> T {
            self.guard
                .update_tables_closure(move |table| table.swap_remove(index))
        }
    }

    impl<'w, 'a, T> WriteGuard<'w, T> {
        pub fn drain<R>(&'a mut self, range: R) -> std::vec::Drain<'a, T>
        where
            R: 'static + Clone + Send + RangeBounds<usize>,
        {
            self.guard.update_tables(Drain { range })
        }
    }

    impl<'w, T: 'static> WriteGuard<'w, T> {
        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Clone + Send + FnMut(&T) -> bool,
        {
            self.guard.update_tables(Retain {
                f,
                _compile_t: std::marker::PhantomData::<fn(*const T)>,
            })
        }
    }
}

pub mod shared {
    use super::*;
    crate::generate_shared_aslock!(Vec<T>);

    impl<'w, T> WriteGuard<'w, T>
    where
        T: 'static + Clone + Send,
    {
        pub fn push(&mut self, value: T) {
            self.guard.update_tables(Push { value })
        }

        pub fn insert(&mut self, index: usize, element: T) {
            self.guard.update_tables(Insert { index, element })
        }
    }

    impl<'w, T> WriteGuard<'w, T> {
        pub fn clear(&mut self) {
            self.guard.update_tables_closure(move |table| table.clear())
        }
        pub fn pop(&mut self) -> Option<T> {
            self.guard.update_tables_closure(move |table| table.pop())
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard
                .update_tables_closure(move |table| table.reserve(additional))
        }

        pub fn reserve_exact(&mut self, additional: usize) {
            self.guard
                .update_tables_closure(move |table| table.reserve_exact(additional))
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard
                .update_tables_closure(move |table| table.shrink_to_fit())
        }

        pub fn truncate(&mut self, len: usize) {
            self.guard
                .update_tables_closure(move |table| table.truncate(len))
        }

        pub fn swap_remove(&mut self, index: usize) -> T {
            self.guard
                .update_tables_closure(move |table| table.swap_remove(index))
        }
    }

    impl<'w, 'a, T> WriteGuard<'w, T> {
        pub fn drain<R>(&'a mut self, range: R) -> std::vec::Drain<'a, T>
        where
            R: 'static + Clone + Send + RangeBounds<usize>,
        {
            self.guard.update_tables(Drain { range })
        }
    }

    impl<'w, T: 'static> WriteGuard<'w, T> {
        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Clone + Send + FnMut(&T) -> bool,
        {
            self.guard.update_tables(Retain {
                f,
                _compile_t: std::marker::PhantomData::<fn(*const T)>,
            })
        }
    }
}

#[cfg(test)]
mod lockless_test {
    use super::*;

    #[test]
    fn push() {
        let lock1 = lockless::AsLockHandle::<i32>::default();
        let lock2 = lock1.clone();
        assert_eq!(lock1.read().unwrap().len(), 0);

        {
            let mut wg = lock1.write().unwrap();
            wg.push(2);
            assert_eq!(wg.len(), 1);
            assert_eq!(lock2.read().unwrap().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*lock1.read().unwrap(), vec![2]);
        assert_eq!(*lock1.write().unwrap(), vec![2]);
        assert_eq!(*lock1.read().unwrap(), vec![2]);
    }

    #[test]
    fn clear() {
        let aslock = lockless::AsLockHandle::<i32>::default();
        assert_eq!(aslock.read().unwrap().len(), 0);

        {
            let aslock2 = aslock.clone();
            let mut wg = aslock.write().unwrap();
            wg.push(2);
            assert_eq!(wg.len(), 1);
            assert_eq!(aslock2.read().unwrap().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*aslock.read().unwrap(), vec![2]);
        assert_eq!(*aslock.write().unwrap(), vec![2]);
        assert_eq!(*aslock.read().unwrap(), vec![2]);

        aslock.write().unwrap().clear();
        assert_eq!(*aslock.read().unwrap(), vec![]);
        assert_eq!(*aslock.write().unwrap(), vec![]);
        assert_eq!(*aslock.read().unwrap(), vec![]);
    }

    #[test]
    fn pop() {
        let table = lockless::AsLockHandle::<i32>::default();
        {
            let mut wg = table.write().unwrap();
            wg.push(2);
            wg.push(3);
            wg.pop();
            wg.push(4);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*table.read().unwrap(), vec![2, 4]);
        assert_eq!(*table.write().unwrap(), vec![2, 4]);
        assert_eq!(*table.read().unwrap(), vec![2, 4]);
    }

    #[test]
    fn indirect_type() {
        let table = lockless::AsLockHandle::<Box<i32>>::default();

        {
            let mut wg = table.write().unwrap();
            wg.push(Box::new(2));
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*table.read().unwrap(), vec![Box::new(2)]);
        assert_eq!(*table.write().unwrap(), vec![Box::new(2)]);
        assert_eq!(*table.read().unwrap(), vec![Box::new(2)]);
    }

    #[test]
    fn reserve() {
        let table = lockless::AsLockHandle::<i32>::default();

        {
            let mut wg = table.write().unwrap();
            wg.reserve(123);
        }

        assert!(table.read().unwrap().capacity() >= 123);
        assert!(table.write().unwrap().capacity() >= 123);
        assert!(table.read().unwrap().capacity() >= 123);
    }

    #[test]
    fn reserve_exact() {
        let table = lockless::AsLockHandle::<i32>::default();

        {
            let mut wg = table.write().unwrap();
            wg.reserve_exact(123);
        }

        assert_eq!(table.read().unwrap().capacity(), 123);
        assert_eq!(table.write().unwrap().capacity(), 123);
        assert_eq!(table.read().unwrap().capacity(), 123);
    }

    #[test]
    fn shrink_to_fit() {
        let table = lockless::AsLockHandle::<i32>::default();

        {
            let mut wg = table.write().unwrap();
            wg.reserve_exact(123);
            wg.push(2);
            wg.push(3);
            wg.shrink_to_fit();
        }

        assert_eq!(table.read().unwrap().capacity(), 2);
        assert_eq!(table.write().unwrap().capacity(), 2);
        assert_eq!(table.read().unwrap().capacity(), 2);
    }

    #[test]
    fn truncate() {
        let table = lockless::AsLockHandle::<i32>::default();

        {
            let mut wg = table.write().unwrap();
            for i in 0..10 {
                wg.push(i);
            }
            wg.truncate(3);
        }

        assert_eq!(*table.read().unwrap(), vec![0, 1, 2]);
        assert_eq!(*table.write().unwrap(), vec![0, 1, 2]);
        assert_eq!(*table.read().unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn swap_remove() {
        let table = lockless::AsLockHandle::<i32>::default();

        {
            let mut wg = table.write().unwrap();
            for i in 0..5 {
                wg.push(i);
            }
            assert_eq!(wg.swap_remove(2), 2);
        }

        assert_eq!(*table.read().unwrap(), vec![0, 1, 4, 3]);
        assert_eq!(*table.write().unwrap(), vec![0, 1, 4, 3]);
        assert_eq!(*table.read().unwrap(), vec![0, 1, 4, 3]);
    }

    #[test]
    fn insert() {
        let table = lockless::AsLockHandle::<i32>::default();
        let table2 = table.clone();

        {
            let mut wg = table.write().unwrap();
            for i in 0..5 {
                wg.push(i);
            }
            wg.insert(2, 10);
            assert_eq!(*wg, vec![0, 1, 10, 2, 3, 4]);
            assert_eq!(*table2.read().unwrap(), vec![]);
        }

        assert_eq!(*table.read().unwrap(), vec![0, 1, 10, 2, 3, 4]);
        assert_eq!(*table.write().unwrap(), vec![0, 1, 10, 2, 3, 4]);
        assert_eq!(*table.read().unwrap(), vec![0, 1, 10, 2, 3, 4]);
    }

    #[test]
    fn retain() {
        let table = lockless::AsLockHandle::<i32>::default();

        {
            let mut wg = table.write().unwrap();
            for i in 0..5 {
                wg.push(i);
            }
            wg.retain(|element| element % 2 == 0);
        }

        assert_eq!(*table.read().unwrap(), vec![0, 2, 4]);
        assert_eq!(*table.write().unwrap(), vec![0, 2, 4]);
        assert_eq!(*table.read().unwrap(), vec![0, 2, 4]);
    }

    #[test]
    fn drain() {
        let table = lockless::AsLockHandle::<i32>::new(vec![]);

        {
            let mut wg = table.write().unwrap();
            for i in 0..5 {
                wg.push(i + 1);
            }
            assert_eq!(wg.drain(1..4).collect::<Vec<_>>(), vec![2, 3, 4]);
        }

        assert_eq!(*table.read().unwrap(), vec![1, 5]);
        assert_eq!(*table.write().unwrap(), vec![1, 5]);
        assert_eq!(*table.read().unwrap(), vec![1, 5]);
    }

    #[test]
    fn lifetimes() {
        let table = lockless::AsLockHandle::<i32>::from_identical(vec![], vec![]);

        {
            let mut wg = table.write().unwrap();
            for i in 0..5 {
                wg.push(i + 1);
            }
            // Switching the order of 'drain' and 'swapped' fails to compile due
            // to the borrow checker, since 'drain' is tied to the lifetime of
            // table.
            let swapped = wg.swap_remove(1);
            let drain = wg.drain(1..4);
            assert_eq!(drain.collect::<Vec<_>>(), vec![5, 3, 4]);
            assert_eq!(swapped, 2);
        }

        assert_eq!(*table.read().unwrap(), vec![1]);
        assert_eq!(*table.write().unwrap(), vec![1]);
        assert_eq!(*table.read().unwrap(), vec![1]);
    }

    #[test]
    fn debug_str() {
        let table = super::lockless::AsLockHandle::<i32>::default();
        {
            table.write().unwrap().push(12);
        }

        assert_eq!(format!("{:?}", table), "AsLockHandle { writer: Writer { num_readers: 1, ops_to_replay: 1, standby_table: [] }, reader: Reader { num_readers: 1, active_table: [12] } }",);
        assert_eq!(
            format!("{:?}", table.write().unwrap()),
            "WriteGuard { swap_active_and_standby: true, num_readers: 1, ops_to_replay: 0, standby_table: [12] }",
        );
        assert_eq!(
            format!("{:?}", table.read().unwrap()),
            "ReadGuard { active_table: [12] }",
        );
    }
}

#[cfg(test)]
mod shared_test {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn push() {
        let lock1 = Arc::new(shared::AsLock::<i32>::default());
        let lock2 = Arc::clone(&lock1);
        assert_eq!(lock1.read().len(), 0);

        {
            let mut wg = lock1.write();
            wg.push(2);
            assert_eq!(wg.len(), 1);
            {
                // Perform check in another thread to avoid potential deadlock
                // (calling both read and write on aslock at the same time).
                thread::spawn(move || {
                    assert_eq!(lock2.read().len(), 0);
                })
                .join()
                .unwrap();
            }
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*lock1.read(), vec![2]);
        assert_eq!(*lock1.write(), vec![2]);
        assert_eq!(*lock1.read(), vec![2]);
    }

    #[test]
    fn clear() {
        let aslock = Arc::new(shared::AsLock::<i32>::default());
        assert_eq!(aslock.read().len(), 0);

        {
            let mut wg = aslock.write();
            wg.push(2);
            assert_eq!(wg.len(), 1);
            {
                // Perform check in another thread to avoid potential deadlock
                // (calling both read and write on aslock at the same time).
                let aslock = Arc::clone(&aslock);
                thread::spawn(move || {
                    assert_eq!(aslock.read().len(), 0);
                })
                .join()
                .unwrap();
            }
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*aslock.read(), vec![2]);
        assert_eq!(*aslock.write(), vec![2]);
        assert_eq!(*aslock.read(), vec![2]);

        aslock.write().clear();
        assert_eq!(*aslock.read(), vec![]);
        assert_eq!(*aslock.write(), vec![]);
        assert_eq!(*aslock.read(), vec![]);
    }

    #[test]
    fn pop() {
        let table = Arc::new(shared::AsLock::<i32>::default());
        {
            let mut wg = table.write();
            wg.push(2);
            wg.push(3);
            wg.pop();
            wg.push(4);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*table.read(), vec![2, 4]);
        assert_eq!(*table.write(), vec![2, 4]);
        assert_eq!(*table.read(), vec![2, 4]);
    }

    #[test]
    fn indirect_type() {
        let table = shared::AsLock::<Box<i32>>::default();

        {
            let mut wg = table.write();
            wg.push(Box::new(2));
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*table.read(), vec![Box::new(2)]);
        assert_eq!(*table.write(), vec![Box::new(2)]);
        assert_eq!(*table.read(), vec![Box::new(2)]);
    }

    #[test]
    fn reserve() {
        let table = Arc::new(shared::AsLock::<i32>::default());

        {
            let mut wg = table.write();
            wg.reserve(123);
        }

        assert!(table.read().capacity() >= 123);
        assert!(table.write().capacity() >= 123);
        assert!(table.read().capacity() >= 123);
    }

    #[test]
    fn reserve_exact() {
        let table = Arc::new(shared::AsLock::<i32>::default());

        {
            let mut wg = table.write();
            wg.reserve_exact(123);
        }

        assert_eq!(table.read().capacity(), 123);
        assert_eq!(table.write().capacity(), 123);
        assert_eq!(table.read().capacity(), 123);
    }

    #[test]
    fn shrink_to_fit() {
        let table = Arc::new(shared::AsLock::<i32>::default());

        {
            let mut wg = table.write();
            wg.reserve_exact(123);
            wg.push(2);
            wg.push(3);
            wg.shrink_to_fit();
        }

        assert_eq!(table.read().capacity(), 2);
        assert_eq!(table.write().capacity(), 2);
        assert_eq!(table.read().capacity(), 2);
    }

    #[test]
    fn truncate() {
        let table = Arc::new(shared::AsLock::<i32>::default());

        {
            let mut wg = table.write();
            for i in 0..10 {
                wg.push(i);
            }
            wg.truncate(3);
        }

        assert_eq!(*table.read(), vec![0, 1, 2]);
        assert_eq!(*table.write(), vec![0, 1, 2]);
        assert_eq!(*table.read(), vec![0, 1, 2]);
    }

    #[test]
    fn swap_remove() {
        let table = shared::AsLock::<i32>::default();

        {
            let mut wg = table.write();
            for i in 0..5 {
                wg.push(i);
            }
            assert_eq!(wg.swap_remove(2), 2);
        }

        assert_eq!(*table.read(), vec![0, 1, 4, 3]);
        assert_eq!(*table.write(), vec![0, 1, 4, 3]);
        assert_eq!(*table.read(), vec![0, 1, 4, 3]);
    }

    #[test]
    fn insert() {
        let table = Arc::new(shared::AsLock::<i32>::default());

        {
            let mut wg = table.write();
            for i in 0..5 {
                wg.push(i);
            }
            wg.insert(2, 10);
            assert_eq!(*wg, vec![0, 1, 10, 2, 3, 4]);
            {
                // Perform check in another thread to avoid potential deadlock
                // (calling both read and write on aslock at the same time).
                let table = Arc::clone(&table);
                thread::spawn(move || {
                    assert_eq!(*table.read(), vec![]);
                })
                .join()
                .unwrap();
            }
        }

        assert_eq!(*table.read(), vec![0, 1, 10, 2, 3, 4]);
        assert_eq!(*table.write(), vec![0, 1, 10, 2, 3, 4]);
        assert_eq!(*table.read(), vec![0, 1, 10, 2, 3, 4]);
    }

    #[test]
    fn retain() {
        let table = shared::AsLock::<i32>::default();

        {
            let mut wg = table.write();
            for i in 0..5 {
                wg.push(i);
            }
            wg.retain(|element| element % 2 == 0);
        }

        assert_eq!(*table.read(), vec![0, 2, 4]);
        assert_eq!(*table.write(), vec![0, 2, 4]);
        assert_eq!(*table.read(), vec![0, 2, 4]);
    }

    #[test]
    fn drain() {
        let table = shared::AsLock::<i32>::new(vec![]);

        {
            let mut wg = table.write();
            for i in 0..5 {
                wg.push(i + 1);
            }
            assert_eq!(wg.drain(1..4).collect::<Vec<_>>(), vec![2, 3, 4]);
        }

        assert_eq!(*table.read(), vec![1, 5]);
        assert_eq!(*table.write(), vec![1, 5]);
        assert_eq!(*table.read(), vec![1, 5]);
    }

    #[test]
    fn lifetimes() {
        let table = shared::AsLock::<i32>::from_identical(vec![], vec![]);

        {
            let mut wg = table.write();
            for i in 0..5 {
                wg.push(i + 1);
            }
            // Switching the order of 'drain' and 'swapped' fails to compile due
            // to the borrow checker, since 'drain' is tied to the lifetime of
            // table.
            let swapped = wg.swap_remove(1);
            let drain = wg.drain(1..4);
            assert_eq!(drain.collect::<Vec<_>>(), vec![5, 3, 4]);
            assert_eq!(swapped, 2);
        }

        assert_eq!(*table.read(), vec![1]);
        assert_eq!(*table.write(), vec![1]);
        assert_eq!(*table.read(), vec![1]);
    }

    #[test]
    fn debug_str() {
        let table = Arc::new(shared::AsLock::<i32>::default());
        {
            table.write().push(12);
        }

        assert_eq!(format!("{:?}", table), "AsLock { num_ops_to_replay: 1 }",);
        assert_eq!(
            format!("{:?}", table.write()),
            "WriteGuard { num_ops_to_replay: 0, standby_table: TableWriteGuard { standby_table: [12] } }",
        );
        assert_eq!(
            format!("{:?}", table.read()),
            "ShardedLockReadGuard { lock: ShardedLock { data: [12] } }",
        );
    }
}
