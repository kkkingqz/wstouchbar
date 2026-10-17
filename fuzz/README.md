# Protocol fuzzing

This directory has its own Cargo workspace. Ordinary workspace builds need only
the pinned stable toolchain. Targets: `session_message`, `daemon_message`,
and `swapchain`.

Install `cargo-fuzz` and a nightly toolchain, then run from this directory:

```sh
cargo +nightly fuzz list
mkdir -p corpus/session_message
cp seeds/session_message/* corpus/session_message/
cargo +nightly fuzz run session_message -- -max_total_time=600
```

The scheduled CI job runs each declared target for at least ten minutes and
uploads crash artifacts. Fuzz execution never opens hardware devices.

Checked-in seeds cover every message kind so fuzzing reaches fields behind magic
and version validation immediately. CI copies them into the writable corpus.
