#! /bin/bash

# Need to escape quotes when calling here to make sure they are retained.
echo_and_run() {
    echo "
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
$*
!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
"
    eval $*  # Use eval to handle strings (Eg RUSTFLAGS=\"--cfg=loom\")
}


echo_and_run sudo cargo clean

# Rudra seems to need to be run with sudo & the environment variables didn't
# seem to stick for me locally. Also running it in sudo creates compilation
# artifacts that can only be cleaned in sudo, so do that here. Run first so
# that user can fill in the sudo password and then leave it to run.
echo_and_run sudo RUDRA_RUNNER_HOME=~/rust/Rudra/rudra_runner \
    ~/rust/Rudra/docker-helper/docker-cargo-rudra \
    ~/rust/active_standby

echo_and_run sudo cargo clean

echo_and_run cargo test --quiet

echo_and_run RUSTFLAGS=\"--cfg loom\" cargo +nightly test --test loom \
    --release --quiet


# tsan requires all libraries to be built with instrumentation, including std,
# not just the local crate.
echo_and_run RUSTFLAGS=\"-Zsanitizer=thread -g\" cargo +nightly bench \
    benchmarks -Z build-std --quiet --target x86_64-unknown-linux-gnu

# tsan doesn't seem to play nice with doc tests, hence the --lib flag.
echo_and_run RUSTFLAGS=\"-Zsanitizer=thread -g\" cargo +nightly test --lib \
    --release --quiet -Z build-std --target x86_64-unknown-linux-gnu

# Miri specifies it should be cleaned beforehand.
echo_and_run cargo clean
echo_and_run cargo +nightly miri test --quiet
echo_and_run cargo clean

echo_and_run cargo +nightly bench --quiet