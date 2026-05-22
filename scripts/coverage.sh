#!/bin/bash
set -e

echo "Building and running tests with coverage instrumentation..."
# Install cargo-llvm-cov if not present
if ! command -v cargo-llvm-cov &> /dev/null; then
    echo "Installing cargo-llvm-cov..."
    cargo install cargo-llvm-cov
fi

cargo llvm-cov --all-features --workspace --html
echo "================================================="
echo "Coverage HTML report generated successfully!"
echo "Open target/llvm-cov/html/index.html in your browser."
