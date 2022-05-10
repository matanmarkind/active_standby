A library for high concurrency reads.

This library is named after the 2 (identical) tables that are held internally:
- Active - this is the table that all Readers view. This table will never be
  write locked, so readers never face contention.
- Standby - this is the table that the Writer mutates. A writer should face
  minimal contention retrieving this table since Readers move to the Active
  table whenever calling `.read()`, so the only contention is long lived
  AsLockReadGuards.

The cost of minimizing contention is:
1. Memory - Internally there are 2 copies of the underlying type the user
   created. This is needed to allow there to always be a table that Readers can
   access out without contention.
2. CPU - The writer must apply all updates twice, once to each table. Lock
   contention for the writer should be less than with a plain RwLock due to
   Readers using the active_table, so it's possible that write times themselves
   will drop.

The usage is meant to be similar to a RwLock. Some of the inspiration came from
the [left_right](https://crates.io/crates/left-right) crate, so feel free to
check that out. The main differences focus on trying to simplify the client
(creating data structures) and user (using data structures) experiences. Because
there are 2 tables which need to be updated, the user does not simply grab a
writer and mutate the table directly. Rather, users provider update functions
(`UpdateTables` trait) and the crate handles replaying these updates on both
tables. There are 2 ways to interact with active_standby data structure:
1. Raw usage of `AsLock<T>`. This provides the `update_tables` interface to a
   user, which takes an update function (or object implementing  `UpdateTables`)
   to update both of the tables.
2. Generating a client which will wrap the `update_tables` interface. This
   provides the user with an interface which imitates a regular
   `RwLockAsLockWriteGuard` (see the `collections` module).

There are 2 flavors of this algorithm that we offer:
1. Lockless - this variant trades off increased performance against changing the
   API to be less like a `RwLock`. This avoids the cost of performing
   synchronization on reads, but this requires that each thread/task that is
   going to access the tables register in advance. Therefore this centers around
   the `AsLockHandle`, which is conceptually similar to `Arc<RwLock>` (meaning a
   separate `AsLockHandle` per thread/task).
2. Sync - this centers around using an `AsLock`, which is meant to feel like a
   `RwLock`. These structs can be shared between threads by cloning & sending an
   `Arc<AsLock>` (like with `RwLock`). The main difference is that instead of
   using `AsLock<Vec<T>>`, you would use `vec::sync::AsLock<T>`. This is
   because both tables must be updated, meaning users can't just dereference
   and mutate the underlying table, and so we provide a wrapper class.

An example of where the sync variant can be preferable is a Tonic service.
There you don't spawn a set of tasks/threads where you can pass each of them a
`lockless::AsLockHandle`. You can use a `sync::AsLock` though.

We provide 2 modules:
1. primitives - The components used to build data structures in the
   active_standby model. Clients usually don't need to utilize the primitives
   and can instead either utilize the pre-made collections, or generate the
   wrapper for their struct using one of the macros and then just implement the
   mutable API for the generated AsLockWriteGuard.
2. collections - Sync and lockless active_standby structs for common
   collections. Each table type has its own `AsLock` (sync) and `AsLockHandle`
   (lockless), as opposed to `RwLock` where you simply pass in the table. This
   is because users can't simply gain write access to the underlying table and
   then mutate it. Instead mutations are done through UpdateTables so that both
   tables will be updated.

Example creating a wrapper class like in `collections`:
```rust
use std::thread::sleep;
use std::time::Duration;
use std::sync::Arc;
use active_standby::primitives::UpdateTables;

// Client's should implement the mutable interface that they want to offer users
// of their active standby data structure. This is not automatically generated.
struct AddOne {}
impl<'a> UpdateTables<'a, i32, ()> for AddOne {
    fn apply_first(&mut self, table: &'a mut i32) {
        *table = *table + 1;
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
    }
}

pub mod sync {
    active_standby::generate_sync_aslock!(i32);

    impl<'w> AsLockWriteGuard<'w> {
        pub fn add_one(&mut self) {
            self.guard.update_tables(super::AddOne {})
        }
    }
}

fn run_lockless() {
    let table = lockless::AsLockHandle::new(0);
    let table2 = table.clone();
    let handle = std::thread::spawn(move || {
        while *table2.read().unwrap() != 1 {
            sleep(Duration::from_micros(100));
        }
    });

    {
        let mut wg = table.write().unwrap();
        wg.add_one();
    }
    handle.join();
}

fn run_sync() {
    let table = Arc::new(sync::AsLock::new(0));
    let table2 = Arc::clone(&table);
    let handle = std::thread::spawn(move || {
        while *table2.read().unwrap() != 1 {
            sleep(Duration::from_micros(100));
        }
    });

    {
        let mut wg = table.write().unwrap();
        wg.add_one();
    }
    handle.join();
}

fn main() {
    run_lockless();
    run_sync();
}
```

If your table has large elements, you may want to save memory by only holding
each element once (e.g. `vec::AsLockHandle<Arc<i32>>`). This can be done
safely so long as no elements of the table are mutated, only inserted and
removed. Using a vector as an example, if you wanted a function that increases
the value of the first element by 1, you would not increment the value behind
the Arc. You would reassign the first element to a new Arc with the incremented
value.

Example of large elements, using the raw `update_tables` interface
(see `UpdateTables` trait):
```rust
use std::sync::Arc;
use active_standby::primitives::UpdateTables;
use active_standby::primitives::lockless::AsLockHandle;

struct UpdateVal {
    index: usize,
    val: Arc<i32>
}
impl<'a> UpdateTables<'a, Vec<Arc<i32>>, ()> for UpdateVal {
    // Mutate the tables, not the values they point to.
    fn apply_first(&mut self, table: &'a mut Vec<Arc<i32>>) {
        table[self.index] = Arc::clone(&self.val);
    }
    fn apply_second(mut self, table: &mut Vec<Arc<i32>>) {
        table[self.index] = self.val;
    }
}

fn main() {
    let table = AsLockHandle::<Vec<Arc<i32>>>::default();
    table.write().unwrap().update_tables_closure(
        |table| table.push(Arc::new(1))
    );
    table.write().unwrap().update_tables(UpdateVal {
        index: 0,
        val: Arc::new(2)
    });
    assert_eq!(*table.read().unwrap(), vec![Arc::new(2)]);
}
```

## Testing
There are a number of tests that come with active_standby (see
tests/tests_script.sh for examples):

[unittests](https://doc.rust-lang.org/book/ch11-01-writing-tests.html)

[benchmarks](https://doc.rust-lang.org/unstable-book/library-features/test.html)

[loom](https://crates.io/crates/loom)

[LLVM Sanitizers](https://doc.rust-lang.org/beta/unstable-book/compiler-flags/sanitizer.html)

[Miri](https://github.com/rust-lang/miri)

[Rudra](https://github.com/sslab-gatech/Rudra)