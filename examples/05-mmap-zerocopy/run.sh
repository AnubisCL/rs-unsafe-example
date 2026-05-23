#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

echo "=== Java mmap 零拷贝示例 ==="
echo ""

mkdir -p out data
javac --release 21 -d out java/MmapZeroCopyDemo.java
java -cp out MmapZeroCopyDemo
