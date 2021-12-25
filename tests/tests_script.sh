#! /bin/bash

echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ cargo clean
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
cargo clean

# Rudra seems to need to be run with sudo & the environment variables didn't
# seem to stick for me locally. Also running it in sudo creates compilation
# artifacts that can only be cleaned in sudo, so do that here. Run first so that
# that user can fill in the sudo password and then leave it to run.
echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ sudo RUDRA_RUNNER_HOME=~/rust/Rudra/rudra_runner ~/rust/Rudra/docker-helper/docker-cargo-rudra ~/rust/active_standby && sudo cargo clean
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
sudo RUDRA_RUNNER_HOME=~/rust/Rudra/rudra_runner ~/rust/Rudra/docker-helper/docker-cargo-rudra ~/rust/active_standby && sudo cargo clean

echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ RUST_BACKTRACE=full cargo test
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
RUST_BACKTRACE=full cargo test

echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ cargo +nightly bench
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
cargo +nightly bench

echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ RUSTFLAGS="--cfg loom" cargo +nightly test --test loom --release
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
RUSTFLAGS="--cfg loom" cargo +nightly test --test loom --release

# tsan requires all libraries to be built with instrumentation, including std,
# not just the local crate.
echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ RUST_BACKTRACE=full RUSTFLAGS="-Zsanitizer=thread -g" RUSTDOCFLAGS=-Zsanitizer=thread cargo +nightly bench -Z build-std --target x86_64-unknown-linux-gnu # tsan
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
RUST_BACKTRACE=full RUSTFLAGS="-Zsanitizer=thread -g" cargo +nightly bench -Z build-std --target x86_64-unknown-linux-gnu # tsan

# tsan doesn't seem to play nice with doc tests, hence the --lib flag.
echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ RUST_BACKTRACE=full RUSTFLAGS="-Zsanitizer=thread -g" RUSTDOCFLAGS=-Zsanitizer=thread cargo +nightly test --lib --release -Z build-std --target x86_64-unknown-linux-gnu # tsan
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
RUST_BACKTRACE=full RUSTFLAGS="-Zsanitizer=thread -g" cargo +nightly test --lib --release -Z build-std --target x86_64-unknown-linux-gnu # tsan

# Miri specifies it should be cleaned beforehand.
echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ cargo clean && cargo +nightly miri test
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
cargo clean && cargo +nightly miri test

echo '
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$ cargo clean
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
'
cargo clean