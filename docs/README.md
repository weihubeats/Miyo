# Miyo 文档索引

> 英文索引见 [en/README.md](en/README.md)。
> 事实源说明：`SPEC.md` 为 M0–M16 历史规范（已冻结）；Tauri 迁移后的架构以
> `ARCHITECTURE.md` 为准；版本变化看仓库根 `CHANGELOG.md`。

## 面向用户

| 文档 | 说明 |
| --- | --- |
| [USER_GUIDE.md](USER_GUIDE.md) / [en/USER_GUIDE.md](en/USER_GUIDE.md) | 用户手册（安装 / 请求 / 环境 / Mock / 测试 / 外观偏好，中英双语） |
| [AGENT.md](AGENT.md) / [en/AGENT.md](en/AGENT.md) | AI Agent 集成（MCP / HTTP 控制面，中英双语） |
| [MIGRATION.md](MIGRATION.md) / [en/MIGRATION.md](en/MIGRATION.md) | Apifox / Postman 迁移（中英双语） |
| [../CHANGELOG.md](../CHANGELOG.md) | 版本变化速览 |

## 面向开发者 / 运维

| 文档 | 说明 |
| --- | --- |
| [ARCHITECTURE.md](ARCHITECTURE.md) / [en/ARCHITECTURE.md](en/ARCHITECTURE.md) | 架构总览（Tauri 2 + Vue 3，含架构图，中英双语） |
| [PLUGIN-SYSTEM.md](PLUGIN-SYSTEM.md) | 插件系统（Core + Plugin 架构：Manifest / SDK / 权限 / 沙箱 / 开发指南） |
| [SPEC.md](SPEC.md) | 历史开发规范（已冻结，仅追溯用） |
| [TAURI_MIGRATION.md](TAURI_MIGRATION.md) | Dioxus → Tauri 迁移记录 |
| [DEPLOY.md](DEPLOY.md) | 打包部署 |
| [RELEASE.md](RELEASE.md) | 版本升级与发版流程 |
| [UPDATER.md](UPDATER.md) | 自动更新说明 |
| [SMOKE_TEST.md](SMOKE_TEST.md) | 手动验收清单 |
| [SCENARIO-FOLDERS.md](SCENARIO-FOLDERS.md) | 集成测试场景目录（多级文件夹 / 批量操作 / 删除策略 / 备份同步） |
| [TEST-SUITES.md](TEST-SUITES.md) | 测试套件与批量执行（执行策略 / 暂停恢复 / 崩溃恢复 / 报告重跑） |
| [MIRROR_CN.md](MIRROR_CN.md) | 国内镜像说明 |
| [MILESTONES.md](MILESTONES.md) / [PROGRESS.md](PROGRESS.md) | 里程碑总览 / 开发进度记录 |

## 官网

- `website/index.html`（English，默认）/ `website/zh/index.html`（中文）：
  静态落地页，随仓库版本同步更新；通过 GitHub Pages 自动部署。
  旧地址 `/en/` 会跳转到英文首页。
