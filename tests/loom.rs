// Run with:
//
// $ RUST_BACKTRACE=full RUSTFLAGS='--cfg loom' cargo +nightly test --test loom
// --release
//
// If there are errors you need additional flags to use checkpointing (see
// docs):
//
// $ RUST_BACKTRACE=full RUSTFLAGS='--cfg loom' cargo +nightly test
// --test="loom" --features="loom/checkpoint" -- --nocapture

#[cfg(loom)]
#[cfg(test)]
mod loom_tests {
    use active_standby::primitives::lockless::SyncWriter;
    use active_standby::primitives::shared::AsLock;
    use active_standby::primitives::UpdateTables;
    use loom::sync::Arc;
    use loom::thread;

    struct AddOne {}
    impl<'a> UpdateTables<'a, i32, ()> for AddOne {
        fn apply_first(&mut self, table: &'a mut i32) {
            *table = *table + 1;
        }
        fn apply_second(mut self, table: &mut i32) {
            self.apply_first(table);
        }
    }
    struct SetZero {}
    impl<'a> UpdateTables<'a, i32, ()> for SetZero {
        fn apply_first(&mut self, table: &mut i32) {
            *table = 0;
        }
        fn apply_second(mut self, table: &mut i32) {
            self.apply_first(table);
        }
    }

    #[test]
    fn lockless_single_thread() {
        loom::model(|| {
            let writer = SyncWriter::<i32>::new(1);
            {
                let mut wg = writer.write();
                wg.update_tables(AddOne {});
            }

            let reader = writer.new_reader();
            let val = thread::spawn(move || *reader.read()).join().unwrap();

            {
                let mut wg = writer.write();
                wg.update_tables(AddOne {});
            }

            assert_eq!(val, 2);

            let reader = writer.new_reader();
            let val = thread::spawn(move || *reader.read()).join().unwrap();
            assert_eq!(val, 3);
        });
    }

    #[test]
    fn shared_single_thread() {
        loom::model(|| {
            let table = Arc::new(AsLock::<i32>::new(1));
            {
                let mut wg = table.write();
                wg.update_tables(AddOne {});
            }

            let table2 = Arc::clone(&table);
            let val = thread::spawn(move || *table2.read()).join().unwrap();

            {
                let mut wg = table.write();
                wg.update_tables(AddOne {});
            }

            assert_eq!(val, 2);

            let table2 = Arc::clone(&table);
            let val = thread::spawn(move || *table2.read()).join().unwrap();
            assert_eq!(val, 3);
        });
    }

    #[test]
    fn lockless_multi_thread() {
        loom::model(|| {
            let writer = SyncWriter::<i32>::new(1);
            {
                let mut wg = writer.write();
                wg.update_tables(AddOne {});
            }

            let reader = writer.new_reader();
            let writer_handle = thread::spawn(move || {
                {
                    let mut wg = writer.write();
                    wg.update_tables(AddOne {});
                    wg.update_tables(AddOne {});
                }
                let mut wg = writer.write();
                wg.update_tables(SetZero {});
            });

            let reader2 = reader.clone();
            let reader_handle = thread::spawn(move || {
                assert_eq!(*reader2.read() % 2, 0);
            });

            assert_eq!(*reader.read() % 2, 0);

            // Cannot join if there are any ReadGuards alive in this thread
            // since this may deadlock.
            assert!(writer_handle.join().is_ok());
            assert!(reader_handle.join().is_ok());

            assert_eq!(*reader.read(), 0);
        });
    }

    #[test]
    fn shared_multi_thread() {
        loom::model(|| {
            let table = Arc::new(AsLock::<i32>::new(1));
            {
                let mut wg = table.write();
                wg.update_tables(AddOne {});
            }

            let table2 = Arc::clone(&table);
            let writer_handle = thread::spawn(move || {
                {
                    let mut wg = table2.write();
                    wg.update_tables(AddOne {});
                    wg.update_tables(AddOne {});
                }
                let mut wg = table2.write();
                wg.update_tables(SetZero {});
            });

            let table2 = Arc::clone(&table);
            let reader_handle = thread::spawn(move || {
                assert_eq!(*table2.read() % 2, 0);
            });

            assert_eq!(*table.read() % 2, 0);

            // Cannot join if there are any ReadGuards alive in this thread
            // since this may deadlock.
            assert!(writer_handle.join().is_ok());
            assert!(reader_handle.join().is_ok());

            assert_eq!(*table.read(), 0);
        });
    }
}
