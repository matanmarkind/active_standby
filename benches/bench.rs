#![feature(test)]

/// Benchmarks for the table. Run via:
///
///     $ cargo +nightly bench
///
/// In practice you may want to run each one separately because running them all
/// at once seems to overtax my computer and slow down some of them (due to CPU
/// heating?).
///
/// Useful to also run this with tsan:
///
///     $ RUST_BACKTRACE=full RUSTFLAGS="-Zsanitizer=thread -g" cargo +nightly bench -Z build-std --target x86_64-unknown-linux-gnu
///
/// In order to do that you need the rust src code:
///
///     $ rustup component add --toolchain nightly rust-src
///
/// See historical benchmark results in "active_standby/benches/records/".
///
// 'test' is a special crate that requires extern crate even though we are using
// rust 2018.
// https://doc.rust-lang.org/nightly/edition-guide/rust-2018/module-system/path-clarity.html
extern crate test;
use active_standby::primitives::lockless::Writer;
use active_standby::primitives::{RwLock, UpdateTables};
use more_asserts::*;
use std::sync::Arc;

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

pub mod lockless {
    active_standby::generate_lockless_aslockhandle!(i32);

    impl<'w> WriteGuard<'w> {
        pub fn add_one(&mut self) {
            self.guard.update_tables(super::AddOne {})
        }
        pub fn set_zero(&mut self) {
            self.guard.update_tables(super::SetZero {})
        }
    }
}

pub mod shared {
    active_standby::generate_shared_aslock!(i32);

    impl<'w> WriteGuard<'w> {
        pub fn add_one(&mut self) {
            self.guard.update_tables(super::AddOne {})
        }
        pub fn set_zero(&mut self) {
            self.guard.update_tables(super::SetZero {})
        }
    }
}

// Updating a plain atomic bool gives a reference point for what kind of speeds
// atomics can achieve. Gives some perspective on the cost we are adding/what a
// lower bound could be.
#[bench]
fn plain_atomicbool(b: &mut test::bench::Bencher) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let ref1 = Arc::new(AtomicBool::new(false));
    let ref2 = Arc::clone(&ref1);

    std::thread::spawn(move || loop {
        ref1.store(!ref1.load(Ordering::SeqCst), Ordering::SeqCst);
    });

    b.iter(|| {
        let val = ref2.load(Ordering::SeqCst);
        let n = test::black_box(true);
        if val == n {
            assert_eq!(val, true);
        } else {
            assert_eq!(val, false);
        }
    });
}

// Test the speed of acquiring write guards when it never has to wait on readers
// to release the table.
#[bench]
fn lockless_wguard_without_rcontention(b: &mut test::bench::Bencher) {
    let writer = Writer::<i32>::new(1);
    b.iter(|| {
        let mut wg = writer.write();
        wg.update_tables(AddOne {});
    });
}
#[bench]
fn shared_wguard_without_rcontention(b: &mut test::bench::Bencher) {
    let table = Arc::new(shared::AsLock::new(1));
    b.iter(|| {
        let mut wg = table.write();
        wg.add_one();
    });
}
#[bench]
fn rwlock_wguard_without_rcontention(b: &mut test::bench::Bencher) {
    let table = Arc::new(RwLock::new(1));
    b.iter(|| {
        let mut wg = table.write().unwrap();
        *wg += 1;
    });
}

#[bench]
fn lockless_wguard_rw_contention(b: &mut test::bench::Bencher) {
    let table = lockless::AsLockHandle::new(1);

    let _reader_handles: Vec<_> = (0..10)
        .map(|_| {
            let table = table.clone();
            std::thread::spawn(move || {
                // Continually grab read guards.
                while *table.read() != 0 {
                    // Hold the read guards to increase the chance of read 'contention'.
                    std::thread::sleep(std::time::Duration::from_micros(10));
                }
            })
        })
        .collect();

    let _writer_handle = {
        let table = table.clone();
        std::thread::spawn(move || loop {
            let mut wg = table.write();
            wg.add_one();
        })
    };

    b.iter(|| {
        let mut wg = table.write();
        wg.add_one();
    });
}
#[bench]
fn shared_wguard_rw_contention(b: &mut test::bench::Bencher) {
    let table = Arc::new(shared::AsLock::new(1));

    let _reader_handles: Vec<_> = (0..10)
        .map(|_| {
            let table = Arc::clone(&table);
            std::thread::spawn(move || {
                // Continually grab read guards.
                while *table.read() != 0 {
                    // Hold the read guards to increase the chance of read 'contention'.
                    std::thread::sleep(std::time::Duration::from_micros(10));
                }
            })
        })
        .collect();

    let _writer_handle = {
        let table = Arc::clone(&table);
        std::thread::spawn(move || loop {
            let mut wg = table.write();
            wg.add_one();
        })
    };

    b.iter(|| {
        let mut wg = table.write();
        wg.add_one();
    });
}
#[bench]
fn rwlock_wguard_rw_contention(b: &mut test::bench::Bencher) {
    let table = Arc::new(RwLock::new(1));

    let _reader_handles: Vec<_> = (0..10)
        .map(|_| {
            let table = Arc::clone(&table);
            std::thread::spawn(move || {
                // Continually grab read guards.
                while *table.read().unwrap() != 0 {
                    // Hold the read guards to increase the chance of read 'contention'.
                    std::thread::sleep(std::time::Duration::from_micros(10));
                }
            })
        })
        .collect();

    let _writer_handle = {
        let table = Arc::clone(&table);
        std::thread::spawn(move || loop {
            let mut wg = table.write().unwrap();
            *wg += 1;
        })
    };

    b.iter(|| {
        let mut wg = table.write().unwrap();
        *wg += 1;
    });
}

// Test the speed of acquiring the ReadGuard when the writer never takes a guard
// are there are no other readers.
#[bench]
fn lockless_rguard_no_contention(b: &mut test::bench::Bencher) {
    let writer = Writer::<i32>::new(1);
    let reader = writer.new_reader();

    b.iter(|| {
        let rg = reader.read();
        assert_eq!(*rg, 1);
    });
}
#[bench]
fn shared_rguard_no_contention(b: &mut test::bench::Bencher) {
    let table = Arc::new(shared::AsLock::new(1));

    b.iter(|| {
        let rg = table.read();
        assert_eq!(*rg, 1);
    });
}

// The main test, since our core guarantee is that reads are always wait free
// regardless of read and write usage.
fn lockless_rguard_rw_contention(b: &mut test::bench::Bencher, num_readers: u32) {
    let table = lockless::AsLockHandle::new(1);

    let _reader_handles: Vec<_> = (0..num_readers)
        .map(|_| {
            let table = table.clone();
            std::thread::spawn(move || {
                // Continually grab read guards.
                while *table.read() != 0 {
                    // Hold the read guards to increase the change of read 'contention'.
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
            })
        })
        .collect();

    let table2 = table.clone();
    let _writer_handle = std::thread::spawn(move || loop {
        let mut wg = table2.write();
        std::thread::sleep(std::time::Duration::from_micros(100));
        wg.add_one();
    });

    b.iter(|| {
        let rg = table.read();
        assert_gt!(*rg, 0);
    });
}

fn shared_rguard_rw_contention(b: &mut test::bench::Bencher, num_readers: u32) {
    let aslock = Arc::new(shared::AsLock::new(1));
    let _reader_handles: Vec<_> = (0..num_readers)
        .map(|_| {
            let aslock = Arc::clone(&aslock);
            std::thread::spawn(move || {
                // Continually grab read guards.
                while *aslock.read() != 0 {
                    // Hold the read guards to increase the change of read 'contention'.
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
            })
        })
        .collect();
    let aslock2 = Arc::clone(&aslock);
    let _writer_handle = std::thread::spawn(move || loop {
        let mut wg = aslock2.write();
        std::thread::sleep(std::time::Duration::from_micros(100));
        wg.add_one();
    });

    b.iter(|| {
        let rg = aslock.read();
        assert_gt!(*rg, 0);
    });
}

fn rwlock_rguard_rw_contention(b: &mut test::bench::Bencher, num_readers: u32) {
    let table = Arc::new(RwLock::new(1));
    let _reader_handles: Vec<_> = (0..num_readers)
        .map(|_| {
            let table = Arc::clone(&table);
            std::thread::spawn(move || {
                // Continually grab read guards.
                while *table.read().unwrap() != 0 {
                    // Hold the read guards to increase the change of read 'contention'.
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
            })
        })
        .collect();

    let _writer_handle = {
        let table = Arc::clone(&table);
        std::thread::spawn(move || loop {
            let mut wg = table.write().unwrap();
            std::thread::sleep(std::time::Duration::from_micros(100));
            *wg += 1;
        });
    };

    b.iter(|| {
        let rg = table.read().unwrap();
        assert_gt!(*rg, 0);
    });
}

// The following section is the main thing we are interested in; how does
// retreiving a read guard to the tables scale with lots of other readers and an
// active writer. The tests are broken down as follows:
// - lockless v. shared
// - writer spinning (constantly write locking and releasing) v. writer grabbing
//   and holding a write guard for 100us.
// - num read threads

#[bench]
fn lockless_rguard_rw_contention_1(b: &mut test::bench::Bencher) {
    lockless_rguard_rw_contention(b, 1);
}
#[bench]
fn lockless_rguard_rw_contention_10(b: &mut test::bench::Bencher) {
    lockless_rguard_rw_contention(b, 10);
}
#[bench]
fn lockless_rguard_rw_contention_20(b: &mut test::bench::Bencher) {
    lockless_rguard_rw_contention(b, 20);
}
#[bench]
fn lockless_rguard_rw_contention_30(b: &mut test::bench::Bencher) {
    lockless_rguard_rw_contention(b, 30);
}

#[bench]
fn shared_rguard_rw_contention_1(b: &mut test::bench::Bencher) {
    shared_rguard_rw_contention(b, 1);
}
#[bench]
fn shared_rguard_rw_contention_10(b: &mut test::bench::Bencher) {
    shared_rguard_rw_contention(b, 10);
}
#[bench]
fn shared_rguard_rw_contention_20(b: &mut test::bench::Bencher) {
    shared_rguard_rw_contention(b, 20);
}
#[bench]
fn shared_rguard_rw_contention_30(b: &mut test::bench::Bencher) {
    shared_rguard_rw_contention(b, 30);
}

#[bench]
fn rwlock_rguard_rw_contention_1(b: &mut test::bench::Bencher) {
    rwlock_rguard_rw_contention(b, 1);
}
#[bench]
fn rwlock_rguard_rw_contention_10(b: &mut test::bench::Bencher) {
    rwlock_rguard_rw_contention(b, 10);
}
#[bench]
fn rwlock_rguard_rw_contention_20(b: &mut test::bench::Bencher) {
    rwlock_rguard_rw_contention(b, 20);
}
#[bench]
fn rwlock_rguard_rw_contention_30(b: &mut test::bench::Bencher) {
    rwlock_rguard_rw_contention(b, 30);
}
