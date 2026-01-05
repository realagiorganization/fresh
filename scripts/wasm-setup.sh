#!/bin/bash
# Setup script for Fresh Editor WASM build environment
# This script installs the required tools for building the WASM version

set -e

echo "=== Fresh Editor WASM Build Setup ==="
echo ""

# Check if Rust is installed
if ! command -v cargo &> /dev/null; then
    echo "Error: Rust/Cargo not found. Please install Rust first:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

echo "Rust version: $(rustc --version)"
echo ""

# Add wasm32 target
echo "Adding wasm32-unknown-unknown target..."
rustup target add wasm32-unknown-unknown

# Install wasm-pack
echo ""
echo "Installing wasm-pack..."
if ! command -v wasm-pack &> /dev/null; then
    cargo install wasm-pack
else
    echo "wasm-pack is already installed: $(wasm-pack --version)"
fi

# Check for wasm-opt (optional)
echo ""
echo "Checking for wasm-opt (optional, for smaller builds)..."
if command -v wasm-opt &> /dev/null; then
    echo "wasm-opt is installed: $(wasm-opt --version)"
else
    echo "wasm-opt not found. For smaller WASM files, install binaryen:"
    echo "  Ubuntu/Debian: sudo apt install binaryen"
    echo "  macOS: brew install binaryen"
fi

echo ""
echo "=== Setup Complete ==="
echo ""
echo "To build:"
echo "  ./scripts/wasm-build.sh"
echo ""
echo "To test locally:"
echo "  cd dist && python3 -m http.server 8080"
echo ""
