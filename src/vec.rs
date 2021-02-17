/// Implementation of Vec for use in the active_standby model.
///
/// Specifically this allows users to call mutating functions on the
/// vec::WriteGuard like they would on a Vec. Functions that return a reference
/// to data owned by the underlying Vec will have different return values
/// because we don't allow tying return values to the underlying data to avoid
/// ever returning a mutable reference which the reader will use to change the
/// table without recording it.
pub mod vec {
    use crate::primitives;
    use crate::primitives::UpdateTables;
    use std::ops::RangeBounds;

    pub struct Reader<T> {
        reader: primitives::Reader<Vec<T>>,
    }

    impl<T> Reader<T> {
        pub fn read(&self) -> ReadGuard<'_, T> {
            ReadGuard {
                guard: self.reader.read(),
            }
        }
    }

    pub struct ReadGuard<'r, T> {
        guard: primitives::ReadGuard<'r, Vec<T>>,
    }

    impl<'w, T> std::ops::Deref for ReadGuard<'w, T> {
        type Target = Vec<T>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    pub struct Writer<T> {
        writer: primitives::Writer<Vec<T>>,
    }

    impl<T> Writer<T>
    where
        T: Clone,
    {
        pub fn new() -> Writer<T> {
            Writer {
                writer: primitives::Writer::<Vec<T>>::new(vec![]),
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
        guard: primitives::WriteGuard<'w, Vec<T>>,
    }

    impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
        type Target = Vec<T>;
        fn deref(&self) -> &Self::Target {
            &*self.guard
        }
    }

    struct Push<T> {
        value: T,
    }
    impl<T> UpdateTables<Vec<T>, ()> for Push<T>
    where
        T: Clone,
    {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            table.push(self.value.clone());
        }
        fn apply_second(self: Box<Self>, table: &mut Vec<T>) {
            table.push(self.value); // Move the value instead of cloning.
        }
    }

    struct Insert<T> {
        index: usize,
        element: T,
    }
    impl<T> UpdateTables<Vec<T>, ()> for Insert<T>
    where
        T: Clone,
    {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            table.insert(self.index, self.element.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut Vec<T>) {
            // Move the value instead of cloning.
            table.insert(self.index, self.element)
        }
    }

    struct Pop {}
    impl<T> UpdateTables<Vec<T>, Option<T>> for Pop {
        fn apply_first(&mut self, table: &mut Vec<T>) -> Option<T> {
            table.pop()
        }
    }

    struct Reserve {
        additional: usize,
    }
    impl<T> UpdateTables<Vec<T>, ()> for Reserve {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            table.reserve(self.additional)
        }
    }

    struct ReserveExact {
        additional: usize,
    }
    impl<T> UpdateTables<Vec<T>, ()> for ReserveExact {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            table.reserve(self.additional)
        }
    }

    struct ShrinkToFit {}
    impl<T> UpdateTables<Vec<T>, ()> for ShrinkToFit {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            table.shrink_to_fit()
        }
    }

    struct Truncate {
        len: usize,
    }
    impl<T> UpdateTables<Vec<T>, ()> for Truncate {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            table.truncate(self.len)
        }
    }

    struct SwapRemove {
        index: usize,
    }
    impl<T> UpdateTables<Vec<T>, T> for SwapRemove {
        fn apply_first(&mut self, table: &mut Vec<T>) -> T {
            table.swap_remove(self.index)
        }
    }

    struct Drain<R>
    where
        R: 'static + Clone + RangeBounds<usize>,
    {
        range: R,
    }
    impl<T, R> UpdateTables<Vec<T>, Vec<T>> for Drain<R>
    where
        R: 'static + Clone + RangeBounds<usize>,
    {
        fn apply_first(&mut self, table: &mut Vec<T>) -> Vec<T> {
            table.drain(self.range.clone()).collect()
        }
        fn apply_second(self: Box<Self>, table: &mut Vec<T>) {
            table.drain(self.range);
        }
    }

    struct Retain<T, F>
    where
        F: 'static + Clone + FnMut(&T) -> bool,
    {
        f: F,
        _compile_t: std::marker::PhantomData<T>,
    }
    impl<T, F> UpdateTables<Vec<T>, ()> for Retain<T, F>
    where
        F: 'static + Clone + FnMut(&T) -> bool,
    {
        fn apply_first(&mut self, table: &mut Vec<T>) {
            table.retain(self.f.clone())
        }
        fn apply_second(self: Box<Self>, table: &mut Vec<T>) {
            table.retain(self.f)
        }
    }

    // Here we reimplement the mutable interface of a Vec.
    impl<'w, T> WriteGuard<'w, T>
    where
        T: 'static + Clone,
    {
        pub fn push(&mut self, value: T) {
            self.guard.update_tables(Push { value })
        }

        pub fn insert(&mut self, index: usize, element: T) {
            self.guard.update_tables(Insert { index, element })
        }
    }

    impl<'w, T> WriteGuard<'w, T> {
        pub fn pop(&mut self) -> Option<T> {
            self.guard.update_tables(Pop {})
        }

        pub fn reserve(&mut self, additional: usize) {
            self.guard.update_tables(Reserve { additional })
        }

        pub fn reserve_exact(&mut self, additional: usize) {
            self.guard.update_tables(ReserveExact { additional })
        }

        pub fn shrink_to_fit(&mut self) {
            self.guard.update_tables(ShrinkToFit {})
        }

        pub fn truncate(&mut self, len: usize) {
            self.guard.update_tables(Truncate { len })
        }

        pub fn swap_remove(&mut self, index: usize) -> T {
            self.guard.update_tables(SwapRemove { index })
        }

        /// Performs the same mutation on the data as Vec::drain, but instead of
        /// returning an iterator to the drained elements, it returns a Vec of
        /// the drained elements.
        ///
        /// This is because the return value from update_tables must own its own
        /// data.
        pub fn drain<R>(&mut self, range: R) -> Vec<T>
        where
            R: 'static + Clone + RangeBounds<usize>,
        {
            self.guard.update_tables(Drain { range })
        }
    }

    impl<'w, T> WriteGuard<'w, T>
    where
        T: 'static,
    {
        pub fn retain<F>(&mut self, f: F)
        where
            F: 'static + Clone + FnMut(&T) -> bool,
        {
            self.guard.update_tables(Retain {
                f,
                _compile_t: std::marker::PhantomData::<T>,
            })
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
            assert_eq!(wg.len(), 1);
            assert_eq!(reader.read().len(), 0);
        }

        // When the write guard is dropped it publishes the changes to the readers.
        assert_eq!(*reader.read(), vec![2]);
        {
            let wg = writer.write();
            assert_eq!(*wg, vec![2]);
        }
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
        {
            let wg = writer.write();
            assert_eq!(*wg, vec![2, 4]);
        }
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
        {
            let wg = writer.write();
            assert_eq!(*wg, vec![Box::new(2)]);
        }
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
        {
            let wg = writer.write();
            assert!(wg.capacity() >= 123);
        }
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
        {
            let wg = writer.write();
            assert_eq!(wg.capacity(), 123);
        }
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
        {
            let wg = writer.write();
            assert_eq!(wg.capacity(), 2);
        }
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
        {
            let wg = writer.write();
            assert_eq!(*wg, vec![0, 1, 2]);
        }
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
            assert_eq!(wg.swap_remove(2), 2);
        }

        assert_eq!(*reader.read(), vec![0, 1, 4, 3]);
        {
            let wg = writer.write();
            assert_eq!(*wg, vec![0, 1, 4, 3]);
        }
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
            assert_eq!(*wg, vec![0, 1, 10, 2, 3, 4]);
            assert_eq!(*reader.read(), vec![]);
        }

        assert_eq!(*reader.read(), vec![0, 1, 10, 2, 3, 4]);
        {
            let wg = writer.write();
            assert_eq!(*wg, vec![0, 1, 10, 2, 3, 4]);
        }
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
        {
            let wg = writer.write();
            assert_eq!(*wg, vec![0, 2, 4]);
        }
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
            assert_eq!(wg.drain(1..4), vec![2, 3, 4]);
        }

        assert_eq!(*reader.read(), vec![1, 5]);
        {
            let wg = writer.write();
            assert_eq!(*wg, vec![1, 5]);
        }
        assert_eq!(*reader.read(), vec![1, 5]);
    }
}
