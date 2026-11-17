#!/usr/bin/env bash
# 确保 Java Debug 2.0 的 bundle.resources 所需 jar 存在且为最新：
#   frontend/src-tauri/resources/java/{miyo-java-agent,miyo-attach-launcher}.jar
#
# tauri-build 在编译期校验 bundle.resources 文件存在，须先预置。
# 构建走 scripts/build_java_debug.sh（Maven，产物同步到 resources/java/）。
# 目标已存在且比 Maven 源新时跳过，增量场景近乎零开销。
#
# 调用方：CI（cargo check 前）、scripts/package-tauri.sh。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RES_DIR="$ROOT/frontend/src-tauri/resources/java"
AGENT_SRC="$ROOT/java/miyo-java-agent/target/miyo-java-agent.jar"
LAUNCHER_SRC="$ROOT/java/miyo-attach-launcher/target/miyo-attach-launcher.jar"
DEST_AGENT="$RES_DIR/miyo-java-agent.jar"
DEST_LAUNCHER="$RES_DIR/miyo-attach-launcher.jar"

need_build=0
if [[ ! -f "$DEST_AGENT" || ! -f "$DEST_LAUNCHER" \
      || ! -f "$AGENT_SRC" || ! -f "$LAUNCHER_SRC" \
      || "$AGENT_SRC" -nt "$DEST_AGENT" || "$LAUNCHER_SRC" -nt "$DEST_LAUNCHER" ]]; then
  need_build=1
fi

if [[ $need_build -eq 1 ]]; then
  echo "==> 构建 Java Debug attach jar（agent + launcher）"
  bash "$ROOT/scripts/build_java_debug.sh"
else
  echo "==> Java Debug attach jar 均为最新（跳过构建）"
fi

ls -la "$RES_DIR"
