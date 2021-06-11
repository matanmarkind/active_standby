#![feature(test)]

/// Benchmarks for the table.
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
// 'test' is a special crate that requires introduction this way even though we
// are using rust 2018.
// https://doc.rust-lang.org/nightly/edition-guide/rust-2018/module-system/path-clarity.html
extern crate test;
use active_standby::primitives::*;
use more_asserts::*;

struct AddOne {}
impl UpdateTables<i32, ()> for AddOne {
    fn apply_first(&mut self, table: &mut i32) {
        *table = *table + 1;
    }
}
struct SetZero {}
impl UpdateTables<i32, ()> for SetZero {
    fn apply_first(&mut self, table: &mut i32) {
        *table = 0;
    }
}

// Test the speed of acquiring write guards when it never has to wait on readers
// to release the table.
#[bench]
fn write_guard_without_contention(b: &mut test::bench::Bencher) {
    let mut writer = Writer::<i32>::new(1);
    b.iter(|| {
        let mut wg = writer.write();
        wg.update_tables(AddOne {});
    });
}

// Test the speed of acquiring write guards when there are many readers taking
// the active_table for short durations.
#[bench]
fn write_guard_with_contention(b: &mut test::bench::Bencher) {
    let mut writer = Writer::<i32>::new(1);
    let _reader_handles: Vec<_> = (0..4)
        .map(|_| {
            let mut reader = writer.new_reader();
            std::thread::spawn(move || {
                // Continually grab read guards. We expect that readers can
                // block the writer, so no point holding the reader for a long
                // time since that will just slow down the benchmark
                while *reader.read() != 0 {}
            })
        })
        .collect();

    b.iter(|| {
        let mut wg = writer.write();
        wg.update_tables(AddOne {});
    });
}

// Test the speed of acquiring the ReadGuard when the writer never takes a guard
// are there are no other readers.
#[bench]
fn read_guard_no_contention(b: &mut test::bench::Bencher) {
    let writer = Writer::<i32>::new(1);
    let mut reader = writer.new_reader();

    b.iter(|| {
        let rg = reader.read();
        assert_eq!(*rg, 1);
    });
}

// Test the speed of acquiring the ReadGuard when there is no writer activity,
// but many other readers.
#[bench]
fn read_guard_read_contention(b: &mut test::bench::Bencher) {
    let writer = Writer::<i32>::new(1);
    let _reader_handles: Vec<_> = (0..20)
        .map(|_| {
            let mut reader = writer.new_reader();
            std::thread::spawn(move || {
                // Continually grab read guards.
                while *reader.read() != 0 {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            })
        })
        .collect();

    let mut reader = writer.new_reader();
    b.iter(|| {
        let rg = reader.read();
        assert_eq!(*rg, 1);
    });
}

// Test the speed of acquiring the ReadGuard when there is no writer activity,
// but many other readers.
#[bench]
fn read_guard_write_contention(b: &mut test::bench::Bencher) {
    let mut writer = SendWriter::<i32>::new(1);
    let mut reader = writer.new_reader();
    let _writer_handle = std::thread::spawn(move || loop {
        let mut wg = writer.write();
        wg.update_tables(AddOne {});
    });

    b.iter(|| {
        let rg = reader.read();
        assert_gt!(*rg, 0);
    });
}

// Test the speed of acquiring the ReadGuard when there is no writer activity,
// but many other readers.
#[bench]
fn read_guard_writehold_contention(b: &mut test::bench::Bencher) {
    let mut writer = SendWriter::<i32>::new(1);
    let mut reader = writer.new_reader();
    let _writer_handle = std::thread::spawn(move || loop {
        let mut wg = writer.write();
        wg.update_tables(AddOne {});

        // Make the write guard long lived to check if its existence causes the
        // reader issues. Can help differentiate cache-ing from deadlock when
        // compared with read_guard_write_contention.
        std::thread::sleep(std::time::Duration::from_millis(10));
    });

    b.iter(|| {
        let rg = reader.read();
        assert_gt!(*rg, 0);
    });
}

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

fn read_guard_readwrite_contention(b: &mut test::bench::Bencher, num_readers: u32) {
    let mut writer = SendWriter::<i32>::new(1);
    let mut reader = writer.new_reader();
    let _reader_handles: Vec<_> = (0..num_readers)
        .map(|_| {
            let mut reader = writer.new_reader();
            std::thread::spawn(move || {
                // Continually grab read guards.
                while *reader.read() != 0 {
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
            })
        })
        .collect();
    let _writer_handle = std::thread::spawn(move || loop {
        let mut wg = writer.write();
        wg.update_tables(AddOne {});
    });

    b.iter(|| {
        let rg = reader.read();
        assert_gt!(*rg, 0);
    });
}

#[bench]
fn read_guard_readwrite_contention_1(b: &mut test::bench::Bencher) {
    read_guard_readwrite_contention(b, 1);
}

#[bench]
fn read_guard_readwrite_contention_10(b: &mut test::bench::Bencher) {
    read_guard_readwrite_contention(b, 10);
}

#[bench]
fn read_guard_readwrite_contention_20(b: &mut test::bench::Bencher) {
    read_guard_readwrite_contention(b, 20);
}

#[bench]
fn read_guard_readwrite_contention_30(b: &mut test::bench::Bencher) {
    read_guard_readwrite_contention(b, 30);
}

#[bench]
fn read_guard_readwrite_contention_40(b: &mut test::bench::Bencher) {
    read_guard_readwrite_contention(b, 40);
}
