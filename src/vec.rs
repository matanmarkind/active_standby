/// Implementation of Vec for use in the active_standby model.
/// vec::AsLockHandle<T>, should function similarly to
/// Arc<RwLock<Vec<T>>>.
pub mod vec {
    use crate::primitives::UpdateTables;
    use std::ops::RangeBounds;

    crate::generate_aslock_handle!(Vec<T>);

    impl<'w, 'a, T> WriteGuard<'w, T>
    where
        T: 'static + Clone + Send,
    {
        pub fn push(&'a mut self, value: T) {
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

            self.guard.update_tables(Push { value })
        }

        pub fn insert(&'a mut self, index: usize, element: T) {
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

            self.guard.update_tables(Insert { index, element })
        }
    }

    impl<'w, T> WriteGuard<'w, T> {
        pub fn clear(&mut self) {
            struct Clear {}

            impl<'a, T> UpdateTables<'a, Vec<T>, ()> for Clear {
                fn apply_first(&mut self, table: &mut Vec<T>) {
                    table.clear()
                }
                fn apply_second(mut self, table: &mut Vec<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(Clear {})
        }
        pub fn pop(&mut self) -> Option<T> {
            struct Pop {}

            impl<'a, T> UpdateTables<'a, Vec<T>, Option<T>> for Pop {
                fn apply_first(&mut self, table: &'a mut Vec<T>) -> Option<T> {
                    table.pop()
                }
                fn apply_second(mut self, table: &mut Vec<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(Pop {})
        }

        pub fn reserve(&mut self, additional: usize) {
            struct Reserve {
                additional: usize,
            }

            impl<'a, T> UpdateTables<'a, Vec<T>, ()> for Reserve {
                fn apply_first(&mut self, table: &'a mut Vec<T>) {
                    table.reserve(self.additional)
                }
                fn apply_second(mut self, table: &mut Vec<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(Reserve { additional })
        }

        pub fn reserve_exact(&mut self, additional: usize) {
            struct ReserveExact {
                additional: usize,
            }

            impl<'a, T> UpdateTables<'a, Vec<T>, ()> for ReserveExact {
                fn apply_first(&mut self, table: &'a mut Vec<T>) {
                    table.reserve(self.additional)
                }
                fn apply_second(mut self, table: &mut Vec<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(ReserveExact { additional })
        }

        pub fn shrink_to_fit(&mut self) {
            struct ShrinkToFit {}

            impl<'a, T> UpdateTables<'a, Vec<T>, ()> for ShrinkToFit {
                fn apply_first(&mut self, table: &'a mut Vec<T>) {
                    table.shrink_to_fit()
                }
                fn apply_second(mut self, table: &mut Vec<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(ShrinkToFit {})
        }

        pub fn truncate(&mut self, len: usize) {
            struct Truncate {
                len: usize,
            }

            impl<'a, T> UpdateTables<'a, Vec<T>, ()> for Truncate {
                fn apply_first(&mut self, table: &'a mut Vec<T>) {
                    table.truncate(self.len)
                }
                fn apply_second(mut self, table: &mut Vec<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(Truncate { len })
        }

        pub fn swap_remove(&mut self, index: usize) -> T {
            struct SwapRemove {
                index: usize,
            }

            impl<'a, T> UpdateTables<'a, Vec<T>, T> for SwapRemove {
                fn apply_first(&mut self, table: &'a mut Vec<T>) -> T {
                    table.swap_remove(self.index)
                }
                fn apply_second(mut self, table: &mut Vec<T>) {
                    self.apply_first(table);
                }
            }

            self.guard.update_tables(SwapRemove { index })
        }
    }

    impl<'w, 'a, T> WriteGuard<'w, T> {
        pub fn drain<R>(&'a mut self, range: R) -> std::vec::Drain<'a, T>
        where
            R: 'static + Clone + Send + RangeBounds<usize>,
        {
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

            self.guard.update_tables(Drain { range })
        }
    }

    impl<'w, T: 'static> WriteGuard<'w, T> {
        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Clone + Send + FnMut(&T) -> bool,
        {
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

            self.guard.update_tables(Retain {
                f,
                _compile_t: std::marker::PhantomData::<fn(*const T)>,
            })
        }
    }
}

#[cfg(test)]
mod test {
    use super::vec::*;

    #[test]
    fn push() {
        let lock1 = AsLockHandle::<i32>::default();
        let lock2 = lock1.clone();
        assert_eq!(lock1.read().len(), 0);

        {
            let mut wg = lock1.write();
            wg.push(2);
            assert_eq!(wg.len(), 1);
            assert_eq!(lock2.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*lock1.read(), vec![2]);
        assert_eq!(*lock1.write(), vec![2]);
        assert_eq!(*lock1.read(), vec![2]);
    }

    #[test]
    fn clear() {
        let aslock = AsLockHandle::<i32>::default();
        assert_eq!(aslock.read().len(), 0);

        {
            let aslock2 = aslock.clone();
            let mut wg = aslock.write();
            wg.push(2);
            assert_eq!(wg.len(), 1);
            assert_eq!(aslock2.read().len(), 0);
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
        let table = AsLockHandle::<i32>::default();
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
        let table = AsLockHandle::<Box<i32>>::default();

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
        let table = AsLockHandle::<i32>::default();

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
        let table = AsLockHandle::<i32>::default();

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
        let table = AsLockHandle::<i32>::default();

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
        let table = AsLockHandle::<i32>::default();

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
        let table = AsLockHandle::<i32>::default();

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
        let table = AsLockHandle::<i32>::default();
        let table2 = table.clone();

        {
            let mut wg = table.write();
            for i in 0..5 {
                wg.push(i);
            }
            wg.insert(2, 10);
            assert_eq!(*wg, vec![0, 1, 10, 2, 3, 4]);
            assert_eq!(*table2.read(), vec![]);
        }

        assert_eq!(*table.read(), vec![0, 1, 10, 2, 3, 4]);
        assert_eq!(*table.write(), vec![0, 1, 10, 2, 3, 4]);
        assert_eq!(*table.read(), vec![0, 1, 10, 2, 3, 4]);
    }

    #[test]
    fn retain() {
        let table = AsLockHandle::<i32>::default();

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
        let table = AsLockHandle::<i32>::new(vec![]);

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
        let table = AsLockHandle::<i32>::from_identical(vec![], vec![]);

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
}
