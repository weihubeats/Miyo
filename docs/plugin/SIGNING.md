# 完整包签名方案（Ed25519）

> 实现：`crates/miyo-plugin/src/package.rs` + `scripts/sign_plugin.mjs`
> + CI 自测（golden vector 跨语言互验）。

## 信任三级

| 级别 | 判定 | 安装行为 |
|---|---|---|
| official | 官方 keyring 任一公钥验签通过 | 直接放行 |
| foreign | 声明了签名但未命中 keyring（第三方自签 / 密钥退役） | 同未签名：需显式信任 |
| unsigned | 未声明签名 | 需显式信任（UI 勾选 + 落库 signed=false） |

## 公钥 keyring 与轮换

```rust
// crates/miyo-plugin/src/package.rs
pub const OFFICIAL_SIGNING_PUBKEYS: &[&str] =
    &["92739639878299e7f459b10fe3ba2c6994559594ff2feb1cbf61eef3d71cd206"];
```

- 轮换：新公钥**追加**表尾（新旧并存期均可验签）→ 旧密钥退役后移除；
- 私钥：只存 CI Secret（建议名 `MIYO_PLUGIN_SIGNING_SEED`，64 位 hex 种子），
  **永不进仓库 / 测试代码**（测试用临时密钥对 roundtrip）；
- 当前官方密钥 #1 生成于 2026-11-18（本机 0600 文件
  `~/.miyo-plugin-signing/miyo-official-1.txt`，使用者需自行配置 Secret）。

## 签名 payload 规范（跨语言确定性）

1. manifest.json 解析为 JSON 对象；`signature` 键**恒参与** payload、值置零
   （128 个 `'0'`——解决签名覆盖自身的鸡生蛋问题，且保证签名前计算与验签时
   重算形状一致）；
2. 按键名字典序递归排序、无空白序列化（serde_json BTreeMap 行为）→ 字节 A；
3. files 行清单 `{path} {sha256} {size}\n`（按 path 字典序）→ 字节 B；
4. `payload = A`（无 files）或 `A || '\n' || B`。

**覆盖范围**：manifest 全部内容（含 permissions / entry / sidecar / files
声明）+ 全部文件摘要——改任何声明或任何文件字节都会破坏签名。

## 跨语言互验（防漂移）

`scripts/testdata/golden_payload.hex`：Node 端生成、Rust 端测试比对
（`signing_payload_matches_node_golden_vector`）。任一侧规范化实现漂移 →
CI 红灯（Node selftest 步骤 + Rust 测试同源输入）。

## 验证时机

- **检视**（plugin_inspect_package）：验签 + 流式全文件 sha256 预验（不解包）；
- **解包**（extract_package_to_dir）：落盘后逐文件复核 + 目录集合比对；
- **启用**（plugin_set_enabled）：manifest integrity + files 清单双重复核，
  结果落库（files_verified / last_verified_at）供详情页展示；
- **手动**（plugin_verify_integrity）：详情页按钮，结果同样落库。

## 工具用法

```bash
node scripts/sign_plugin.mjs keygen                  # 生成密钥对
MIYO_PLUGIN_SIGNING_SEED=<seed> node scripts/sign_plugin.mjs sign <dir>
node scripts/sign_plugin.mjs verify <dir>            # 完整性 + 验签
node scripts/sign_plugin.mjs selftest                # golden vector 自检
MIYO_PLUGIN_SIGNING_SEED=<seed> bash scripts/pack_plugin.sh <dir>   # 打包+签名
```
