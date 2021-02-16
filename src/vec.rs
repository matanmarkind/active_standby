/// Implementation of Vec for use in the active_standby model.
///
/// Specifically this allows users to call mutating functions on the
/// vec::WriteGuard like they would on a Vec, and we internally handle converting
/// this to a struct which implements UpdateTables.
pub mod vec {
    use crate::primitives;
    use std::ops::{Bound, RangeBounds};

    /// This section with bounds is all because "the trait
    /// `std::ops::RangeBounds` cannot be made into an object". So we can't have
    /// a Box<dyn RangeBounds.
    fn bound_ref_to_bound<T: Clone>(bound: &Bound<&T>) -> Bound<T> {
        match bound {
            Bound::Excluded(limit) => Bound::Excluded((*limit).clone()),
            Bound::Included(limit) => Bound::Included((*limit).clone()),
            Bound::Unbounded => Bound::Unbounded,
        }
    }
    fn bound_to_bound_ref<T: Clone>(bound: &Bound<T>) -> Bound<&T> {
        match bound {
            Bound::Excluded(limit) => Bound::Excluded(&limit),
            Bound::Included(limit) => Bound::Included(&limit),
            Bound::Unbounded => Bound::Unbounded,
        }
    }
    struct MyBounds {
        pub start: Bound<usize>,
        pub end: Bound<usize>,
    }
    impl RangeBounds<usize> for MyBounds {
        fn start_bound(&self) -> Bound<&usize> {
            bound_to_bound_ref(&self.start)
        }
        fn end_bound(&self) -> Bound<&usize> {
            bound_to_bound_ref(&self.end)
        }
    }
    impl Clone for MyBounds {
        fn clone(&self) -> Self {
            MyBounds {
                start: self.start,
                end: self.end,
            }
        }
    }

    enum UpdateOp<T> {
        Push(T),
        Pop,
        Reserve(usize),
        ReserveExact(usize),
        ShrinkToFit,
        SwapRemove(usize),
        Insert(usize, T),
        Truncate(usize),
        Retain(Box<dyn 'static + FnMut(&T) -> bool>),
        Drain(MyBounds),
    }

    struct Update<T> {
        update_op: UpdateOp<T>,
    }

    impl<T> primitives::UpdateTables<Vec<T>> for Update<T>
    where
        T: Clone,
    {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            match &mut self.update_op {
                UpdateOp::Push(element) => table.push(element.clone()),
                UpdateOp::Pop => {
                    table.pop();
                }
                UpdateOp::Reserve(additional) => table.reserve(*additional),
                UpdateOp::ReserveExact(additional) => table.reserve_exact(*additional),
                UpdateOp::ShrinkToFit => table.shrink_to_fit(),
                UpdateOp::SwapRemove(index) => {
                    table.swap_remove(*index);
                }
                UpdateOp::Insert(index, element) => table.insert(*index, element.clone()),
                UpdateOp::Truncate(len) => table.truncate(*len),
                UpdateOp::Retain(f) => {
                    table.retain(f);
                }
                UpdateOp::Drain(bounds) => {
                    table.drain(bounds.clone());
                }
            };
        }

        fn apply_second(mut self: Box<Self>, table: &mut Vec<T>) {
            match self.update_op {
                UpdateOp::Push(element) => table.push(element),
                UpdateOp::Insert(index, element) => table.insert(index, element),
                UpdateOp::Drain(bounds) => {
                    table.drain(bounds);
                }
                _ => Self::apply_first(&mut self, table),
            };
        }
    }

    pub struct Writer<T>
    where
        T: Clone,
    {
        writer: primitives::Writer<Vec<T>, Update<T>>,
        reader: Reader<T>,
    }

    pub struct WriteGuard<'w, T>
    where
        T: Clone,
    {
        guard: primitives::WriteGuard<'w, Vec<T>, Update<T>>,
    }

    pub struct Reader<T> {
        reader: primitives::Reader<Vec<T>>,
    }

    pub struct ReadGuard<'r, T> {
        guard: primitives::ReadGuard<'r, Vec<T>>,
    }

    impl<T> Reader<T> {
        pub fn read(&self) -> ReadGuard<'_, T> {
            ReadGuard {
                guard: self.reader.read(),
            }
        }
    }
    impl<T> Clone for Reader<T> {
        fn clone(&self) -> Self {
            Reader {
                reader: primitives::Reader::<Vec<T>>::clone(&self.reader),
            }
        }
    }

    impl<'w, T> std::ops::Deref for ReadGuard<'w, T> {
        type Target = Vec<T>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    impl<T> Writer<T>
    where
        T: Clone,
    {
        pub fn new() -> Writer<T> {
            let writer = primitives::Writer::<Vec<T>, Update<T>>::new(Vec::<T>::new());
            let reader = Reader {
                reader: writer.new_reader(),
            };
            Writer { writer, reader }
        }

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

        pub fn read(&self) -> ReadGuard<'_, T> {
            self.reader.read()
        }
    }

    // Here we reimplement the mutable interface of a Vec.
    impl<'w, T> WriteGuard<'w, T>
    where
        T: Clone,
    {
        pub fn push(&mut self, value: T) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::Push(value),
            });
        }

        pub fn pop(&mut self) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::Pop,
            });
        }
        pub fn reserve(&mut self, additional: usize) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::Reserve(additional),
            });
        }
        pub fn reserve_exact(&mut self, additional: usize) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::ReserveExact(additional),
            });
        }
        pub fn shrink_to_fit(&mut self) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::ShrinkToFit,
            });
        }
        pub fn swap_remove(&mut self, index: usize) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::SwapRemove(index),
            });
        }
        pub fn insert(&mut self, index: usize, element: T) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::Insert(index, element),
            });
        }
        pub fn truncate(&mut self, len: usize) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::Truncate(len),
            });
        }
        pub fn retain<F: 'static + FnMut(&T) -> bool>(&mut self, f: F) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::Retain(Box::new(f)),
            });
        }
        pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) {
            self.guard.update_tables(Update {
                update_op: UpdateOp::Drain(MyBounds {
                    start: bound_ref_to_bound(&range.start_bound()),
                    end: bound_ref_to_bound(&range.end_bound()),
                }),
            });
        }
    }
}

#[cfg(test)]
mod test {
    use super::vec::*;

    #[test]
    fn push() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();
        assert_eq!(reader.read().len(), 0);

        {
            let mut wg = writer.write();
            wg.push(2);
            assert_eq!(reader.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*reader.read(), vec![2]);
        assert_eq!(*writer.read(), vec![2]);
        writer.write();
        assert_eq!(*writer.read(), vec![2]);
        assert_eq!(*reader.read(), vec![2]);
    }

    #[test]
    fn pop() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            wg.push(2);
            wg.push(3);
            wg.pop();
            wg.push(4);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*reader.read(), vec![2, 4]);
        assert_eq!(*writer.read(), vec![2, 4]);
        writer.write();
        assert_eq!(*writer.read(), vec![2, 4]);
        assert_eq!(*reader.read(), vec![2, 4]);
    }

    #[test]
    fn indirect_type() {
        let mut writer = Writer::<Box<i32>>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            wg.push(Box::new(2));
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*reader.read(), vec![Box::new(2)]);
        assert_eq!(*writer.read(), vec![Box::new(2)]);
        writer.write();
        assert_eq!(*writer.read(), vec![Box::new(2)]);
        assert_eq!(*reader.read(), vec![Box::new(2)]);
    }

    #[test]
    fn reserve() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            wg.reserve(123);
        }

        assert!(reader.read().capacity() >= 123);
        assert!(writer.read().capacity() >= 123);
        writer.write();
        assert!(writer.read().capacity() >= 123);
        assert!(reader.read().capacity() >= 123);
    }

    #[test]
    fn reserve_exact() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            wg.reserve_exact(123);
        }

        assert_eq!(reader.read().capacity(), 123);
        assert_eq!(writer.read().capacity(), 123);
        writer.write();
        assert_eq!(writer.read().capacity(), 123);
        assert_eq!(reader.read().capacity(), 123);
    }

    #[test]
    fn shrink_to_fit() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            wg.reserve_exact(123);
            wg.push(2);
            wg.push(3);
            wg.shrink_to_fit();
        }

        assert_eq!(reader.read().capacity(), 2);
        assert_eq!(writer.read().capacity(), 2);
        writer.write();
        assert_eq!(writer.read().capacity(), 2);
        assert_eq!(reader.read().capacity(), 2);
    }

    #[test]
    fn truncate() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            for i in 0..10 {
                wg.push(i);
            }
            wg.truncate(3);
        }

        assert_eq!(*reader.read(), vec![0, 1, 2]);
        assert_eq!(*writer.read(), vec![0, 1, 2]);
        writer.write();
        assert_eq!(*writer.read(), vec![0, 1, 2]);
        assert_eq!(*reader.read(), vec![0, 1, 2]);
    }

    #[test]
    fn swap_remove() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            for i in 0..5 {
                wg.push(i);
            }
            wg.swap_remove(2)
        }

        assert_eq!(*reader.read(), vec![0, 1, 4, 3]);
        assert_eq!(*writer.read(), vec![0, 1, 4, 3]);
        writer.write();
        assert_eq!(*writer.read(), vec![0, 1, 4, 3]);
        assert_eq!(*reader.read(), vec![0, 1, 4, 3]);
    }

    #[test]
    fn insert() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            for i in 0..5 {
                wg.push(i);
            }
            wg.insert(2, 10);
        }

        assert_eq!(*reader.read(), vec![0, 1, 10, 2, 3, 4]);
        assert_eq!(*writer.read(), vec![0, 1, 10, 2, 3, 4]);
        writer.write();
        assert_eq!(*writer.read(), vec![0, 1, 10, 2, 3, 4]);
        assert_eq!(*reader.read(), vec![0, 1, 10, 2, 3, 4]);
    }

    #[test]
    fn retain() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            for i in 0..5 {
                wg.push(i);
            }
            wg.retain(|element| element % 2 == 0);
        }

        assert_eq!(*reader.read(), vec![0, 2, 4]);
        assert_eq!(*writer.read(), vec![0, 2, 4]);
        writer.write();
        assert_eq!(*writer.read(), vec![0, 2, 4]);
        assert_eq!(*reader.read(), vec![0, 2, 4]);
    }

    #[test]
    fn drain() {
        let mut writer = Writer::<i32>::new();
        let reader = writer.new_reader();

        {
            let mut wg = writer.write();
            for i in 0..5 {
                wg.push(i + 1);
            }
            wg.drain(1..4);
        }

        assert_eq!(*reader.read(), vec![1, 5]);
        assert_eq!(*writer.read(), vec![1, 5]);
        writer.write();
        assert_eq!(*writer.read(), vec![1, 5]);
        assert_eq!(*reader.read(), vec![1, 5]);
    }
}
