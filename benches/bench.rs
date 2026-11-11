#![feature(test)]

/// Benchmarks for the table. (example invocations in tests/tests_script.sh)
///
/// See the comment at the top of `mod benchmarks` for what each benchmark
/// measures.
///
/// Useful to also run this with tsan. In order to do that you need the rust src
/// code:
///
///     $ rustup component add --toolchain nightly rust-src
///
/// See historical benchmark results in "active_standby/benches/records/".
///
// 'test' is a special crate that requires extern crate even though we are using
// rust 2018.
// https://doc.rust-lang.org/nightly/edition-guide/rust-2018/module-system/path-clarity.html
extern crate test;
use active_standby::UpdateTables;
use more_asserts::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;

struct AddOne {}
impl<'a> UpdateTables<'a, i32, ()> for AddOne {
    fn apply_first(&mut self, table: &'a mut i32) {
        *table += 1;
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

pub mod lockless {
    active_standby::generate_lockless_aslockhandle!(i32);

    impl<'w> AsLockWriteGuard<'w> {
        pub fn add_one(&mut self) {
            self.guard.update_tables(super::AddOne {})
        }
        pub fn set_zero(&mut self) {
            self.guard.update_tables(super::SetZero {})
        }
    }
}

pub mod sync {
    active_standby::generate_sync_aslock!(i32);

    impl<'w> AsLockWriteGuard<'w> {
        pub fn add_one(&mut self) {
            self.guard.update_tables(super::AddOne {})
        }
        pub fn set_zero(&mut self) {
            self.guard.update_tables(super::SetZero {})
        }
    }
}

/// Background threads which each run their closure in a loop until this is
/// dropped, at which point they are stopped and joined. This keeps benches from
/// leaking threads that would steal CPU from later benches.
struct Background {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Background {
    fn new() -> Background {
        Background {
            stop: Arc::new(AtomicBool::new(false)),
            threads: vec![],
        }
    }

    fn spawn(&mut self, mut f: impl FnMut() + Send + 'static) {
        let stop = Arc::clone(&self.stop);
        self.threads.push(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                f();
            }
        }));
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            t.join().unwrap();
        }
    }
}

#[cfg(test)]
mod benchmarks {
    use super::*;

    // Every benchmark uses the same setup: N background threads reading
    // nonstop, N in {1, 6, 14}. Each compares lockless v. sync v. a plain
    // `parking_lot::RwLock`.
    //
    // - `rguard_{variant}_{N}`: time to take a read guard, while 1 more
    //   background thread writes nonstop. This is the workload active_standby
    //   exists for. Lockless readers only write their own epoch counter, so they
    //   share nothing writable and reads stay flat as N grows. Sync (and a plain
    //   RwLock) has every reader do an atomic RMW on the same lock word, so that
    //   cache line bounces between cores. Sync reads never block on the writer
    //   though, which a plain RwLock does.
    // - `wguard_{variant}_{N}`: time to take a write guard and apply an update;
    //   the bench thread is the only writer. This shows the cost of each
    //   design: the lockless writer waits for every reader to leave the old
    //   table, sync only waits for readers still on the standby table, and a
    //   plain RwLock waits for all readers.
    //
    // For meaningful results keep N + 2 <= the number of cores.
    enum Op {
        Read,
        Write,
    }

    fn contention<H: Send + 'static>(
        b: &mut test::bench::Bencher,
        num_readers: u32,
        op: Op,
        // Called once per thread, since lockless needs a handle per thread.
        make_handle: impl Fn() -> H,
        read: fn(&H) -> i32,
        write: fn(&H),
    ) {
        let mut bg = Background::new();
        for _ in 0..num_readers {
            let h = make_handle();
            bg.spawn(move || {
                test::black_box(read(&h));
            });
        }

        let h = make_handle();
        match op {
            Op::Read => {
                let w = make_handle();
                bg.spawn(move || write(&w));
                b.iter(|| assert_gt!(read(&h), 0));
            }
            Op::Write => b.iter(|| write(&h)),
        }
    }

    fn lockless(b: &mut test::bench::Bencher, num_readers: u32, op: Op) {
        let table = lockless::AsLockHandle::new(1);
        contention(
            b,
            num_readers,
            op,
            || table.clone(),
            |h| *h.read(),
            |h| h.write().add_one(),
        );
    }

    fn sync(b: &mut test::bench::Bencher, num_readers: u32, op: Op) {
        let table = Arc::new(sync::AsLock::new(1));
        contention(
            b,
            num_readers,
            op,
            || Arc::clone(&table),
            |h| *h.read(),
            |h| h.write().add_one(),
        );
    }

    fn rwlock(b: &mut test::bench::Bencher, num_readers: u32, op: Op) {
        let table = Arc::new(RwLock::new(1));
        contention(
            b,
            num_readers,
            op,
            || Arc::clone(&table),
            |h| *h.read(),
            |h| *h.write() += 1,
        );
    }

    macro_rules! bench {
        ($name:ident, $variant:ident, $op:ident, $n:expr) => {
            #[bench]
            fn $name(b: &mut test::bench::Bencher) {
                $variant(b, $n, Op::$op);
            }
        };
    }

    bench!(rguard_lockless_01, lockless, Read, 1);
    bench!(rguard_lockless_06, lockless, Read, 6);
    bench!(rguard_lockless_14, lockless, Read, 14);
    bench!(rguard_sync_01, sync, Read, 1);
    bench!(rguard_sync_06, sync, Read, 6);
    bench!(rguard_sync_14, sync, Read, 14);
    bench!(rguard_rwlock_01, rwlock, Read, 1);
    bench!(rguard_rwlock_06, rwlock, Read, 6);
    bench!(rguard_rwlock_14, rwlock, Read, 14);

    bench!(wguard_lockless_01, lockless, Write, 1);
    bench!(wguard_lockless_06, lockless, Write, 6);
    bench!(wguard_lockless_14, lockless, Write, 14);
    bench!(wguard_sync_01, sync, Write, 1);
    bench!(wguard_sync_06, sync, Write, 6);
    bench!(wguard_sync_14, sync, Write, 14);
    bench!(wguard_rwlock_01, rwlock, Write, 1);
    bench!(wguard_rwlock_06, rwlock, Write, 6);
    bench!(wguard_rwlock_14, rwlock, Write, 14);
}
