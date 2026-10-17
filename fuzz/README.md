# Protocol fuzzing

This directory has its own Cargo workspace. Ordinary workspace builds need only
the pinned stable toolchain. Decoder targets are added in H2.

Install `cargo-fuzz` and a nightly toolchain, then run from this directory:

```sh
cargo +nightly fuzz list
cargo +nightly fuzz run session_message -- -max_total_time=600
```

The scheduled CI job runs each declared target for at least ten minutes and
uploads crash artifacts. Fuzz execution never opens hardware devices.
