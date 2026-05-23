#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$SCRIPT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${CYAN}╔═══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║       Zero-Copy Benchmark — Build & Run                     ║${NC}"
echo -e "${CYAN}╚═══════════════════════════════════════════════════════════════╝${NC}"
echo ""

# ---- Step 1: 编译 Rust 动态链接库 (workspace root) ----
echo -e "${YELLOW}[Step 1/3] 编译 Rust dylib (cargo build --release)...${NC}"
cd "$PROJECT_ROOT"
cargo build --release -p rust-hasher
cd "$SCRIPT_DIR"
echo ""

# workspace 模式下 dylib 在项目根 target/ 下
DYLIB="$PROJECT_ROOT/target/release/librust_hasher.dylib"
if [ ! -f "$DYLIB" ]; then
    echo -e "${RED}错误: 找不到 $DYLIB${NC}"
    exit 1
fi
echo -e "${GREEN}  ✓ $DYLIB${NC}"

echo -e "  导出符号:"
nm -gU "$DYLIB" | grep "rust_" || true
echo ""

# ---- Step 2: 编译 Java ----
echo -e "${YELLOW}[Step 2/3] 编译 Java 源码 (javac)...${NC}"
mkdir -p java/out
javac --enable-preview --release 21 -d java/out java/src/*.java
echo -e "${GREEN}  ✓ Java 编译完成${NC}"
echo ""

# ---- Step 3: 创建 data 目录 & 运行 ----
mkdir -p data

echo -e "${YELLOW}[Step 3/3] 运行基准测试...${NC}"
echo -e "  JVM 参数: --enable-preview --enable-native-access=ALL-UNNAMED"
echo ""

java \
    --enable-preview \
    --enable-native-access=ALL-UNNAMED \
    -Djava.library.path="$PROJECT_ROOT/target/release" \
    -cp java/out \
    BenchmarkRunner "$@"

echo ""
echo -e "${GREEN}基准测试完成!${NC}"
