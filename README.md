# wstouchbar

Touch Bar for Apple T2 MacBooks on Linux: a small root daemon (`touchbard`) that
owns the hardware and a user session that renders declarative button sets
driven by WebAssembly logic modules.

A rewrite of [kkkingqz/touchbar](https://github.com/kkkingqz/touchbar)
(branch `t2-appletbdrm`, a fork of
[cameroncooper/touchbar](https://github.com/cameroncooper/touchbar)).
The rewrite follows [docs/architecture](docs/architecture/README.md) (in Russian).
Stage 1 builds the pure engine; the daemon, renderer and WebAssembly runtime
are planned for later stages. The core currently implements profiles, instance
lifecycles, touch gestures, panels and key bindings with per-surface layer stacks. It schedules effect requests and
subscriptions with cancellation, coalescing, quotas and deterministic timers. Panel widgets share their owner's logic
and state; the renderer supplies explicit slot and panel bounds.

## Development

Install Rust with rustup; the repository pins the toolchain and its components.
From the repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI also runs a scheduled and manually triggered test job with
`PROPTEST_CASES=100000`. The property tests are added during stage 1.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
