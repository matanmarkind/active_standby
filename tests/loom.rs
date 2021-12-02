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
    use active_standby::primitives::lockless::Writer;
    use active_standby::primitives::shared::AsLock;
    use active_standby::primitives::UpdateTables;
    use loom::sync::{Arc, Condvar, LockResult, Mutex, MutexGuard};
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

    pub fn wait_while<'a, T, F>(
        cv: &Condvar,
        mut guard: MutexGuard<'a, T>,
        mut condition: F,
    ) -> LockResult<MutexGuard<'a, T>>
    where
        F: FnMut(&mut T) -> bool,
    {
        while condition(&mut *guard) {
            guard = cv.wait(guard)?;
        }
        Ok(guard)
    }

    #[test]
    fn lockless_single_thread() {
        loom::model(|| {
            let writer = Writer::<i32>::new(1);
            {
                let mut wg = writer.write().unwrap();
                wg.update_tables(AddOne {});
            }

            let reader = writer.new_reader();
            let val = thread::spawn(move || *reader.read()).join().unwrap();

            {
                let mut wg = writer.write().unwrap();
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
        // Loom requires models to be deterministic. Without CondVar to
        // determine the actual order of execution, we can't promise the order
        // that spawned threads will run in. In this example, either the writer
        // or reader could run before the other. Loom catches this because the
        // epoch counter shows a different value when trying to claim a
        // WriteGuard. https://github.com/tokio-rs/loom/issues/233.
        loom::model(|| {
            let writer = Writer::<i32>::new(0);

            let cond_cv = Arc::new((Mutex::new(0), Condvar::new()));
            let reader = writer.new_reader();
            let writer_handle = {
                let cond_cv = Arc::clone(&cond_cv);

                thread::spawn(move || {
                    let (cond, cv) = &*cond_cv;

                    let mut step_num;
                    {
                        let mut wg = Some(writer.write().unwrap());
                        wg.as_mut().unwrap().update_tables(AddOne {});

                        *cond.lock().unwrap() += 1;
                        cv.notify_all();
                        step_num = wait_while(&cv, cond.lock().unwrap(), |step| *step < 2).unwrap();

                        // Write while holding the ReadGuard.
                        wg.as_mut().unwrap().update_tables(AddOne {});

                        // Make sure to drop wg before notifying the reader.
                    }

                    *step_num += 1;
                    cv.notify_all();
                })
            };

            let (cond, cv) = &*cond_cv;
            {
                let rg;
                {
                    let mut step_num =
                        wait_while(&cv, cond.lock().unwrap(), |step| *step < 1).unwrap();

                    // Grab reader and while holding the WriteGuard.
                    rg = reader.read();
                    assert_eq!(*rg, 0);

                    *step_num += 1;
                    cv.notify_all();
                }
                let _step_num = wait_while(&cv, cond.lock().unwrap(), |step| *step < 3);
                // Retaining the old reader will retain the old value.
                assert_eq!(*rg, 0);
            }
            // Grabbing a new reader will show the newly published value.
            assert_eq!(*reader.read(), 2);

            // Cannot join if there are any ReadGuards alive in this thread
            // since this may deadlock.
            assert!(writer_handle.join().is_ok());
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
