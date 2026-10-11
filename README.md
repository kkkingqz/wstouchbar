# wstouchbar

Touch Bar for Apple T2 MacBooks on Linux: a small root daemon (`touchbard`) that
owns the hardware and a user session that renders declarative button sets
driven by WebAssembly logic modules.

A rewrite of [kkkingqz/touchbar](https://github.com/kkkingqz/touchbar)
(branch `t2-appletbdrm`, a fork of
[cameroncooper/touchbar](https://github.com/cameroncooper/touchbar)).
The rewrite follows [docs/architecture](docs/architecture/README.md) (in Russian).
Stage 1 builds the pure engine; the daemon, renderer and WebAssembly runtime
are planned for later stages. The core implements profiles, instance lifecycles,
touch gestures, panels and
key bindings with per-surface layer stacks. It schedules effect requests and
subscriptions with cancellation, coalescing, quotas and deterministic timers.
Panel widgets share their owner's logic and state. Scoped persistence and full
checkpoints preserve state across recreation and replay.

The virtual-time testkit provides fake logic, effects and layout, checks all
twelve engine invariants after each input, and reproduces JSON regression
scenarios. See [testing](docs/testing.md) and [stage 1 acceptance](docs/validation/stage-1.md).

## Development

Install Rust with rustup; the repository pins the toolchain and its components.
From the repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI also runs a scheduled and manually triggered test job with
`PROPTEST_CASES=100000` in release mode. To run the invariant property locally:

```sh
PROPTEST_CASES=100000 cargo test -p touchbar-testkit --test properties --release --locked -- --nocapture
```

The project has no runnable daemon or user service yet; current tests use only
the pure engine and fake executors.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
