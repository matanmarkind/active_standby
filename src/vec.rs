/// Implementation of Vec for use in the active_standby model.
///
/// Specifically this allows users to call mutating functions on the
/// VecWriteGuard like they would on a Vec, and we internally handle converting
/// this to a struct which implements UpdateTables.
use crate::primitives;
use crate::primitives::UpdateTables;
use std::any::Any;

pub struct Writer<T> {
    writer: primitives::Writer<Vec<T>>,
}

pub struct WriteGuard<'w, T> {
    guard: primitives::WriteGuard<'w, Vec<T>>,
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
        Writer {
            writer: primitives::Writer::<Vec<T>>::new_from_empty(Vec::<T>::new()),
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

impl<'w, T> std::ops::Deref for WriteGuard<'w, T> {
    type Target = primitives::WriteGuard<'w, Vec<T>>;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

// Here we reimplement the mutable interface of a Vec.
struct Push<T> {
    value: T,
}
impl<T> UpdateTables<Vec<T>> for Push<T>
where
    T: Clone,
{
    fn apply_first(&mut self, table: &mut Vec<T>) -> Box<dyn Any> {
        table.push(self.value.clone());
        Box::new(())
    }
    fn apply_second(self: Box<Self>, table: &mut Vec<T>) -> Box<dyn Any> {
        table.push(self.value); // Move the value instead of cloning.
        Box::new(())
    }
}
impl<'w, T> WriteGuard<'w, T>
where
    T: 'static + Clone,
{
    pub fn push(&mut self, val: T) {
        self.guard.update_tables(Box::new(Push { value: val }));
    }
}

struct Pop {}
impl<T> UpdateTables<Vec<T>> for Pop {
    fn apply_first(&mut self, table: &mut Vec<T>) -> Box<dyn Any> {
        table.pop();
        Box::new(())
    }
}
impl<'w, T> WriteGuard<'w, T> {
    pub fn pop(&mut self) {
        self.guard.update_tables(Box::new(Pop {}));
    }
}

#[cfg(test)]
mod test {
    use super::*;

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
    }
}