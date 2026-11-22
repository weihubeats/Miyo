---
name: push
description: 推送 Miyo 代码到远程：按 CI 同款门禁预检（cargo fmt/check/clippy/test、miyo-tauri、前端 lint/test/build），主题化提交后推送并监控 CI 直到绿。当用户说「推送」「push」「提交推送」「推代码」时使用。仅用于本仓库（weihubeats/Miyo）。
---

# Miyo 推送流程

用户说「推送」（可带范围：`推送 前端`、`推送 全部`；缺省全部未提交改动）时执行本流程。
目标：**本地跑过 = CI 绿**，杜绝「推送后 CI 挂了再补一刀」。

## 0. 盘点变更

```bash
git status --short
git log --oneline -5        # 对齐仓库提交信息风格（约定式中文：type(scope): 描述）
git branch --show-current   # 本仓库直接提交并推送 main
```

- 按「主题」分组提交（一个 feature/fix 一个 commit），不要一坨全塞；
- **提交前检查清单**：临时文件、密钥/令牌、`node_modules`、构建产物不许入库；
- 纯文档 / 网站 / 配置改动可跳过 §1–§3 的重检查（CI 仍会全量跑）。

## 1. 后端根 workspace 门禁（CI job: test）

任一步失败先修复，**不要带病推送**：

```bash
cargo fmt --all                      # 直接应用格式化（新增 .rs 必跑——CI 会 --check 拒收）
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

已知红线（契约测试会拦）：

- 新增 Rust 模型 / IPC 响应体：字段一律 **snake_case**，禁止 `#[serde(rename_all = "camelCase")]`；
- 新增 Tauri 插件命令：必须同时登记 `generate_handler!`、`build.rs COMMANDS`、
  `permissions/default.toml` 三处（`permissions_contract_test` 双向校验）。

## 2. miyo-tauri 独立工作区（CI job: miyo-tauri）

仅当 `crates/miyo-tauri` 或 `crates/miyo-*` 有改动时执行（独立 workspace，根 workspace 的
命令覆盖不到）：

```bash
cd crates/miyo-tauri
cargo fmt --all && cargo fmt --all -- --check   # CI 不查 fmt，但保持一致
cargo check
cargo test
```

## 3. 前端门禁（CI job: frontend）

仅当 `frontend/**` 有改动时执行：

```bash
bash scripts/ensure-mcp-bin.sh       # build 前置：tauri-build 校验 externalBin 存在
cd frontend
npm run lint                         # eslint
npm test                             # vitest
npm run build                        # vue-tsc + vite（类型检查在这里）
```

仅当后端 crate 或 `frontend/src-tauri` 有改动时，补跑（慢，数分钟，但 CI 必跑）：

```bash
cd frontend/src-tauri && cargo check && cargo check --release
```

## 4. 提交与推送

```bash
git add <按主题分组的路径>     # 避免 git add -A 盲加
git commit                      # 约定式中文信息：type(scope): 描述 + 要点列表
git push origin main
```

## 5. 推送后监控 CI（必做，不放任自流）

```bash
curl -s "https://api.github.com/repos/weihubeats/Miyo/actions/runs?branch=main&per_page=1" \
  | python3 -c "import json,sys;r=json.load(sys.stdin)['workflow_runs'][0];print(r['name'],r['status'],r['conclusion'],r['html_url'])"
```

- 确认新推送的 `head_sha` 对应的 run 已 `in_progress`；
- 每 2~3 分钟轮询一次直到 `completed`（三个 job：test / miyo-tauri / frontend）；
- `conclusion: success` → 向用户汇报通过；
- `failure` → 定位失败 job：
  `curl -s https://api.github.com/repos/weihubeats/Miyo/actions/runs/<id>/jobs`
  拉取失败步骤日志 → 修复 → **从头重走本流程**（重新跑门禁 + 提交 + 推送）。

## 踩坑记录（历史上真实发生，勿重蹈）

1. 新增 `.rs` 文件忘 fmt → CI format job 红（本次 skill 的由来）；
2. `list_project_stats` 注册了 handler 但漏登权限清单 → 运行时被拒、功能静默失效
   （本地测试全绿也发现不了，权限契约测试因此而生）；
3. IPC 响应体加 camelCase 重命名 → 前端读到 undefined、统计显示 0
   （序列化契约测试因此而生）；
4. 递归组件的 provide/inject 键写在 `<script setup>` 体内 → 每实例各一份 Symbol
   永不匹配（键必须放模块级 `<script>`）。
5. 本地 `cargo clippy --all-targets` 不带 `-D warnings` → 测试代码里的未用变量
   只产生本地 warning，CI `-D warnings` 直接编译拒绝（root Test 步骤 ubuntu
   实证）；本地门禁必须照抄 CI 的 `-D warnings` 尾巴。
