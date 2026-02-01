#!/bin/bash
# Build script for textamp - works on macOS and Linux

set -e

echo "Building textamp (release)..."
cargo build --release

# Get the binary path
BINARY="target/release/textamp"

if [ -f "$BINARY" ]; then
    FULL_PATH="$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")"
    SIZE=$(ls -lh "$BINARY" | awk '{print $5}')

    echo ""
    echo "Build complete!"
    echo "Binary: $FULL_PATH"
    echo "Size:   $SIZE"
else
    echo "Error: Binary not found at $BINARY"
    exit 1
fi
