#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "═══════════════════════════════════════════════════"
echo "  Geek Auth Bytecode Injection - Build All"
echo "═══════════════════════════════════════════════════"

# Step 1: Build Java Agent
echo ""
echo "[1/3] Building Java Agent..."
cd "$SCRIPT_DIR/java-agent"
mvn clean package -q
echo "  ✓ inject-agent-1.0.0.jar"

# Step 2: Build Spring Boot App
echo ""
echo "[2/3] Building Spring Boot App..."
cd "$SCRIPT_DIR/java-app"
mvn clean package -q -DskipTests
echo "  ✓ auth-app-1.0.0.jar"

# Step 3: Build Rust Injector
echo ""
echo "[3/3] Building Rust Injector..."
cd "$SCRIPT_DIR/rust-injector"
cargo build --release -q 2>/dev/null || cargo build -q
echo "  ✓ rust-injector"

# 复制 artifacts 到统一目录
echo ""
echo "Copying artifacts..."
mkdir -p "$SCRIPT_DIR/target"
cp "$SCRIPT_DIR/java-agent/target/inject-agent-1.0.0.jar" "$SCRIPT_DIR/target/"
cp "$SCRIPT_DIR/java-app/target/auth-app-1.0.0.jar" "$SCRIPT_DIR/target/"

echo ""
echo "═══════════════════════════════════════════════════"
echo "  Build Complete!"
echo "═══════════════════════════════════════════════════"
echo ""
echo "Usage:"
echo "  # Terminal 1: Start Spring Boot app"
echo "  cd $SCRIPT_DIR/target"
echo "  java -jar auth-app-1.0.0.jar"
echo ""
echo "  # Terminal 2: Run Rust injector"
echo "  cd $SCRIPT_DIR/target"
echo "  $SCRIPT_DIR/rust-injector/target/release/rust-injector inject-agent-1.0.0.jar"
echo ""
echo "  Or use -javaagent at startup:"
echo "  java -javaagent:inject-agent-1.0.0.jar -jar auth-app-1.0.0.jar"
