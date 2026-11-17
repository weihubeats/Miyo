#!/usr/bin/env bash
# 打包第三方插件：目录 → .miyo（tar.gz）。
# 用法：scripts/pack_plugin.sh <插件目录> [输出文件]
# 约定：目录内必须有 manifest.json（字段规范见 docs/PLUGIN-SYSTEM.md）；
# 包内条目为相对路径（不允许 .. / 绝对路径 / 符号链接——安装端会拒绝）。
set -euo pipefail

dir="${1:?用法: pack_plugin.sh <插件目录> [输出文件]}"
out="${2:-${dir%/}.miyo}"

if [[ ! -f "$dir/manifest.json" ]]; then
  echo "错误：$dir/manifest.json 不存在" >&2
  exit 1
fi

# -H 不跟随符号链接、只归档常规文件（安装端对非常规条目一律拒绝）
tar --format=ustar -czf "$out" -C "$dir" .
echo "已打包：$out"
