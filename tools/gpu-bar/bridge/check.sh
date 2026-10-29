#!/bin/sh
set -eu
: "${CARGO_TARGET_DIR:?run through cargo-slot niltempus}"
bridge=tools/gpu-bar/bridge
cargo fmt --manifest-path "$bridge/Cargo.toml" --check
cargo test --manifest-path "$bridge/Cargo.toml" --locked
cargo clippy --manifest-path "$bridge/Cargo.toml" --all-targets --locked -- -D warnings
cargo build --manifest-path "$bridge/Cargo.toml" --lib --locked
# Linux native libraries required by this pinned Rust static library.
cc -std=c11 -Wall -Wextra -Werror -I"$bridge/include" "$bridge/tests/c_abi.c" \
    "$CARGO_TARGET_DIR/debug/libgpu_bar_bridge.a" \
    -ldl -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc \
    -o "$CARGO_TARGET_DIR/debug/gpu-bar-bridge-c-abi"
"$CARGO_TARGET_DIR/debug/gpu-bar-bridge-c-abi"
cargo fmt --manifest-path tools/gpu-bar/dma/Cargo.toml --check
cargo test --manifest-path tools/gpu-bar/dma/Cargo.toml --locked
cargo clippy --manifest-path tools/gpu-bar/dma/Cargo.toml --all-targets --locked -- -D warnings
