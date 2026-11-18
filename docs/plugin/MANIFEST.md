# Manifest 2.0 规范

> 真源：`crates/miyo-plugin/src/manifest.rs`（deny_unknown_fields，前端类型为
> ts-rs 镜像）。`sdk_version` 保持 1；2.0 字段全部为**可选增量**，v1 插件
> 零改动可装。未知字段一律拒绝。

## 字段全集

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| id | string | ✅ | reverse-DNS（2-4 段，小写字母开头） |
| name / description | string | ✅ | ≤ 96 / 512 字符；内置插件建议配 name_key/description_key |
| version | string | ✅ | 严格 x.y.z（每段 0-999，拒绝 v 前缀 / 预发布） |
| kind | enum | ✅ | `ui` / `ui-sidecar`（后者必须携带 sidecar 配置） |
| min_app_version | string | ✅ | 严格 x.y.z |
| sdk_version | number | ✅ | 当前支持 1 |
| authors | string[] | – | ≤ 10 |
| icon | string | – | IconName 或 data:image/*（≤ 16 KiB） |
| entry | string | ui ✅ | 相对路径，仅 .html；必须在 files 清单内（如有） |
| nav | PluginNavItem[] | – | ≤ 8；route 以 / 开头、无穿越段、label(_key) 二选一 |
| permissions | string[] | – | ≤ 16，封闭目录逐条命中（见 PERMISSIONS.md） |
| runtime_deps | PluginRuntimeDep[] | – | ≤ 8；kind ≤ 32 字符 |
| plugin_deps | map<id, 最低版本> | – | ≤ 8；禁自依赖；启用 / 安装时解析（deps.rs） |
| config_schema | object | – | ≤ 32 键；type ∈ string/number/boolean/enum |
| integrity | sha256 hex | – | manifest.json 字节摘要（安装端现算落库） |
| signature | 128 hex | – | Ed25519（payload 规范见 SIGNING.md） |
| **files** | FileEntry[] | – | **2.0 新增**：全文件清单（path/sha256/size），声明即强制 |
| **sidecar** | object | ui-sidecar ✅ | executable / protocol(≥1) / max_memory_mb(1-4096) / restart_policy(off \| on-crash) |
| **platforms** | string[] | – | windows/macos/linux（可带 -arm64/-x64）；未声明 = 全平台 |

## files 清单语义（声明即强制）

1. 包内文件集合必须与清单**完全一致**（清单外文件拒绝、缺失拒绝）；
2. 逐文件 sha256 + size 比对（检视流式预验 → 解包落盘复核 → 启用再复核）；
3. `entry` / `sidecar.executable` 必须在清单内；`manifest.json` 本身由
   integrity（安装端现算）单独保护；
4. 路径规则与解包白名单一致：`[A-Za-z0-9._-/]`、禁绝对路径 / `..` / 反斜杠；
5. manifest.files 内 sha256 必须小写 hex。

## 示例（ui-sidecar）

```json
{
  "id": "com.example.echo-sidecar",
  "name": "Echo Sidecar 示例",
  "version": "0.1.0",
  "kind": "ui-sidecar",
  "sdk_version": 1,
  "min_app_version": "0.0.60",
  "entry": "index.html",
  "sidecar": {
    "executable": "bin/echo-sidecar",
    "protocol": 1,
    "max_memory_mb": 256,
    "restart_policy": "on-crash"
  },
  "platforms": ["windows", "macos", "linux"],
  "permissions": ["std.storage", "runtime.process.start", "runtime.process.stop"],
  "files": [
    { "path": "index.html", "sha256": "…", "size": 1024 },
    { "path": "bin/echo-sidecar", "sha256": "…", "size": 1400000 }
  ]
}
```

> 注：`files` 清单由签名工具 / `scripts/sign_plugin.mjs sign` 自动生成，
> 手写只需在签名步骤前省略。
