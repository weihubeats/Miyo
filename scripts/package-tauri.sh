#!/usr/bin/env bash
# Miyo 打包脚本（Tauri 2 版：macOS / Linux / Windows）
#
# 用法:
#   scripts/package-tauri.sh            # 构建 + 打包当前平台（产物在 target/release/bundle/）
#
# 依赖:Node 22+（npm ci）、Rust stable（tauri CLI 经 npm 调用）
# 产物（bundle 目录）:
#   macOS:   Miyo.app / Miyo.dmg / Miyo-macos-*.zip
#   Linux:   Miyo.deb / Miyo.AppImage
#   Windows: Miyo.msi / Miyo-setup.exe

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FRONTEND="$ROOT/frontend"

echo "==> 安装前端依赖 (npm ci)"
(cd "$FRONTEND" && npm ci)

echo "==> 预置 miyo-mcp 侧载二进制（bundle.externalBin）"
"$ROOT/scripts/ensure-mcp-bin.sh"
bash "$ROOT/scripts/ensure-java-jars.sh"

echo "==> tauri build（含 vite 前端构建 + Rust release 编译 + 平台 bundle）"
(cd "$FRONTEND" && npm run tauri build)

BUNDLE="$ROOT/target/release/bundle"
echo "==> 打包完成，产物目录：$BUNDLE"
ls -la "$BUNDLE"/* 2>/dev/null || true
