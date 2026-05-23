#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Panama + Rust 零拷贝示例 ==="
echo ""

# Step 1: 编译 Rust dylib
echo "[1/3] 编译 Rust dylib..."
cd "$PROJECT_ROOT"
cargo build --release -p rust-offheap
cd "$SCRIPT_DIR"
echo ""

DYLIB="$PROJECT_ROOT/target/release/librust_offheap.dylib"
if [ ! -f "$DYLIB" ]; then
    echo "错误: 找不到 $DYLIB"
    exit 1
fi
echo "  ✓ $(basename $DYLIB)"
nm -gU "$DYLIB" | grep "rust_" || true
echo ""

# Step 2: 编译 Java
echo "[2/3] 编译 Java..."
mkdir -p out
javac --enable-preview --release 21 -d out java/PanamaRustDemo.java
echo "  ✓ 编译完成"
echo ""

# Step 3: 运行
echo "[3/3] 运行..."
echo ""
java \
    --enable-preview \
    --enable-native-access=ALL-UNNAMED \
    -Djava.library.path="$PROJECT_ROOT/target/release" \
    -cp out \
    PanamaRustDemo

echo ""
echo "完成"
