#!/bin/bash
cd /mnt/e/cc/src-tauri
echo "Building application..."
cargo build 2>&1 | tail -20
echo "Build complete. Running application..."
timeout 30 cargo run 2>&1 | grep -A5 -B5 "DEBUG\|ERROR\|panic\|Generate"