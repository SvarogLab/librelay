#!/bin/sh
# Builds and runs one fuzz target: fuzz/run.sh <target> [seconds, default 60].
#
# `cargo fuzz run` writes its corpus, its crashes and a lockfile into fuzz/; built and run from
# here, all three stay in the build directory instead.
set -eu

target=$1
seconds=${2:-60}
fuzz_dir=$(cd "$(dirname "$0")" && pwd)
out=${CARGO_TARGET_DIR:-$fuzz_dir/../target}/fuzz
host=$(rustc +nightly -vV | sed -n 's/^host: //p')

CARGO_RESOLVER_LOCKFILE_PATH=$out/Cargo.lock \
    cargo +nightly fuzz build --fuzz-dir "$fuzz_dir" --target-dir "$out" "$target"
mkdir -p "$out/corpus/$target" "$out/artifacts/$target"
exec "$out/$host/release/$target" -max_total_time="$seconds" -timeout=5 \
    -artifact_prefix="$out/artifacts/$target/" "$out/corpus/$target"
