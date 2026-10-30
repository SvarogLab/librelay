# Code style

## Formatting

`rustfmt` with the repository's `rustfmt.toml`; `cargo fmt --all -- --check` passes. Markdown passes
`markdownlint-cli2` with `.markdownlint-cli2.jsonc`.

## Lints

`cargo clippy --all-targets --all-features -- -D warnings` passes on Linux, and on the Windows
target too (see the README's Testing section for the environment it needs), where the `client`
module compiles to nothing. Crate-wide lints live in `[lints]` in `Cargo.toml`; a threshold goes
into `clippy.toml` only with a comment naming the code it is for.

## Layout

- A module with submodules lives in `name.rs` beside a `name/` directory, never in `name/mod.rs`
  (`clippy::mod_module_files` enforces it).
- `client` is the Linux side and gated on `unix`; `server` is the Windows side; `mux` is what both
  share, implied by either.
- Tests sit in a `tests` module at the bottom of the file they test, or in `name/tests.rs` when they
  outgrow it. A test that drives the crate only from outside, through its public API or its built
  binary, goes in `tests/`.

## Naming

Types and functions say what they are in the relay: the `client` runs on Linux, the `server` is
the Windows process it spawns, the `mux` the yamux connection between them over the server's
stdio, and a `stream` one unit of work carried over it. Test names are sentences stating the
behavior they hold.

## Errors

- Each module that can fail has its own `thiserror` type; errors cross a module boundary through a
  `From` impl or a `#[source]` field, not by stringifying.
- `unwrap` belongs in tests. Elsewhere, an `expect` states the invariant that makes it unreachable.

## Comments

Code says what it does; a comment says why, when the reason lives outside the code: a quirk of
Windows, of WSL or of systemd, with the behavior it forces.

## Dependencies

Requirements name the current release, at the same majors the relays using this crate depend on,
so nothing gets built twice. A test-only dependency on a crate's unstable API is acceptable; a
runtime one is not.
