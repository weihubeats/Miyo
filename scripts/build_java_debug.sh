#!/usr/bin/env bash
# Miyo Java Debug 2.0 构建脚本：Agent + Attach Launcher → 单一 dist 目录。
#
# 产物：
#   java/dist/miyo-java-agent.jar        动态注入目标 JVM 的调试 Agent
#   java/dist/miyo-attach-launcher.jar   Attach sidecar CLI（Miyo 以子进程调用）
#
# 两个 jar 均编译为 Java 8 字节码（可注入 JDK 8+ 任意 HotSpot 目标进程）。
# Tauri 打包：scripts/package 时复制到 frontend/src-tauri/resources/java/。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/java/dist"

echo "==> 构建 miyo-java-agent（shade：ASM 重定位）"
(cd "$ROOT/java/miyo-java-agent" && mvn -q package -DskipTests)

echo "==> 构建 miyo-attach-launcher"
(cd "$ROOT/java/miyo-attach-launcher" && mvn -q package -DskipTests)

mkdir -p "$DIST"
cp "$ROOT/java/miyo-java-agent/target/miyo-java-agent.jar" "$DIST/"
cp "$ROOT/java/miyo-attach-launcher/target/miyo-attach-launcher.jar" "$DIST/"

echo "==> 产物"
ls -la "$DIST"

# Tauri bundle 资源目录（存在则同步，CI/发布构建使用）
TAURI_RES="$ROOT/frontend/src-tauri/resources/java"
if [ -d "$ROOT/frontend/src-tauri" ]; then
  mkdir -p "$TAURI_RES"
  cp "$DIST/miyo-java-agent.jar" "$DIST/miyo-attach-launcher.jar" "$TAURI_RES/"
  echo "==> 已同步到 Tauri 资源目录: $TAURI_RES"
fi
