# wstouchbar

Touch Bar for Apple T2 MacBooks on Linux: a small root daemon (`touchbard`) that
owns the hardware and a user session that renders declarative button sets
driven by WebAssembly logic modules.

A rewrite of [kkkingqz/touchbar](https://github.com/kkkingqz/touchbar)
(branch `t2-appletbdrm`, a fork of
[cameroncooper/touchbar](https://github.com/cameroncooper/touchbar)).
The rewrite follows [docs/architecture](docs/architecture/README.md) (in Russian).
The pure engine, WebAssembly runtime and user session are implemented. The session
loads plugins, renders declarative controls, executes effects and can open Wayland
simulator windows. The hardware daemon and GNOME integration are planned for the
next stages. See [testing](docs/testing.md), [stage 3 acceptance](docs/validation/stage-3.md)
and [architecture](docs/architecture/README.md).

## Development

Install Rust with rustup; the repository pins the toolchain and its components.
Native build dependencies are `build-essential`, `pkg-config`, `clang` and
`libpipewire-0.3-dev` on Ubuntu. A running PipeWire server is not required for tests.
From the repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI also runs a scheduled and manually triggered test job with
`PROPTEST_CASES=100000` and `PROPTEST_CONTROLS_CASES=10000` in release mode. To run the invariant property locally:

```sh
PROPTEST_CASES=100000 cargo test -p touchbar-testkit --test properties --release --locked -- --nocapture
```

## Simulator

```sh
cargo xtask build-plugins
cargo build --workspace --bins --locked
target/debug/touchbar-session --simulator --desktop=fake --plugins target/plugins
target/debug/touchbarctl --simulator status
```

The simulator uses real PipeWire audio; add `--effects=fake` for simulated effects.
Mouse clicks act as touches, holding VOL opens a transient panel, Right Alt acts
as Fn and Ctrl+T switches the fake desktop theme. Keys and shortcuts are logged.
Several windows are supported with repeated `--surface`, for example
`--surface touchbar=2008x60@6.2ppmm --surface window:test=1200x160@4ppmm`.
The simulator has its own control socket and state, so it can run beside the
primary session. No service is installed by these commands.

`touchbarctl render` works offline. Environment checks are explicit:
`cargo xtask test-env gpu|dbus|pipewire`; they use private contexts and services.
See [tooling](docs/architecture/11-tooling.md) for commands and the control protocol.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
