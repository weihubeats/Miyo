#!/usr/bin/env bash
# 打包第三方插件：目录 → .miyo（tar.gz）。
# 用法：scripts/pack_plugin.sh <插件目录> [输出文件]
# 约定：目录内必须有 manifest.json（字段规范见 docs/PLUGIN-SYSTEM.md）；
# 包内条目为相对路径（不允许 .. / 绝对路径 / 符号链接——安装端会拒绝）。
#
# 签名（可选）：设置 MIYO_PLUGIN_SIGNING_SEED 后，打包前自动生成全文件
# 清单（manifest.files）并写入 Ed25519 签名（scripts/sign_plugin.mjs sign）。
# 未设置时跳过签名——产物走「未签名 + 用户显式信任」安装通道。
set -euo pipefail

dir="${1:?用法: pack_plugin.sh <插件目录> [输出文件]}"
out="${2:-${dir%/}.miyo}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ! -f "$dir/manifest.json" ]]; then
  echo "错误：$dir/manifest.json 不存在" >&2
  exit 1
fi

# 签名（可选）：生成 files 清单 + Ed25519 签名写回 manifest.json
if [[ -n "${MIYO_PLUGIN_SIGNING_SEED:-}" ]]; then
  node "$ROOT/scripts/sign_plugin.mjs" sign "$dir"
else
  echo "（未设置 MIYO_PLUGIN_SIGNING_SEED，跳过签名——产物为未签名包）"
fi

# -H 不跟随符号链接、只归档常规文件（安装端对非常规条目一律拒绝）
tar --format=ustar -czf "$out" -C "$dir" .
echo "已打包：$out"
