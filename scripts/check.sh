#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
./native/build.sh
./native/test.sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
./target/release/rosorah-studio --self-check
