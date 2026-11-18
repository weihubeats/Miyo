# Miyo 插件架构 2.0

## 总体结构

```mermaid
graph TB
    subgraph MiyoCore["Miyo Core（不堆积业务）"]
        PM[Plugin Manager<br/>生命周期编排 / 更新 / 回滚]
        PR[Plugin Registry<br/>SQLite 持久化 + manifest 2.0 校验]
        SDK[Plugin SDK<br/>会话凭证 / runtime.* / 权限门禁]
        PERM[Permission Manager<br/>18 项封闭目录 + 后端强制]
        ST[Plugin Storage / Log<br/>行级隔离 + 滚动日志]
        EB[Plugin Event Bus<br/>前端总线 + supervisor 事件]
        UIH[Plugin UI Host<br/>iframe 沙箱 + RPC v1 加固]
        SUP[Sidecar Supervisor<br/>miyo-sidecar crate]
        SES[Session Registry<br/>std 族凭证]
        ATT[Attach Manager<br/>Java Debug 专用]
    end

    subgraph Plugins["插件形态"]
        UI[UI Plugin<br/>iframe only]
        UIS[UI+Sidecar Plugin<br/>iframe + 常驻进程]
        BUILTIN[Builtin Plugin<br/>随应用打包，同规则]
        EXT[External Plugin<br/>第三方 .miyo]
    end

    UI --> UIH --> SDK --> PERM
    UIS --> UIH
    UIS --> SUP
    BUILTIN --> PM
    EXT --> PM --> PR
    SDK --> SES
    SDK --> ST
    SDK --> EB
```

## 命令族与权限强制矩阵

| 命令族 | 数量 | 后端强制 |
|---|---|---|
| plugin_* 生命周期 / 市场 | 26 | 启用态 + 安装流程校验（签名 / 兼容 / 依赖） |
| plugin_data_* / record_log | 5 | **会话凭证**（plugin_session_create 签发，收权即吊销） |
| plugin_read_asset | 1 | 受管目录 + 拒 symlink + 扩展名白名单 |
| plugin_sidecar_* | 4 | runtime.process.start / stop 权限 |
| plugin_update / rollback / versions | 3 | 安装流程校验 + 快照消费制 |
| java_*（v1 Runtime） | 14 | java.* 子权限（discover/attach/inspect/invoke 按族） |
| java_attach_* / java_agent_*（v2） | 14 | 同上（28/28 全覆盖） |

## 分层职责（与 AGENTS.md 一致）

- `miyo-plugin`：纯逻辑（清单校验 / 包解包 / 签名验签 / 权限目录 / 依赖解析 /
  兼容性）——不做 IPC / DB；
- `miyo-storage`：持久化（插件注册表 / KV / 日志 / 版本历史 / 权限审计）；
- `miyo-sidecar`：常驻插件进程监督（协议 / 握手 / 心跳 / 崩溃恢复 / 平台加固）；
- `miyo-java-attach`：Java Attach 编排（launcher sidecar / Agent 会话表）；
- `miyo-tauri`：命令编排层（门禁 + 审计 + view 组装）——不写业务规则；
- 前端 `plugins/`：SDK / manager / RPC host（防御纵深第一道，**不作为**
  安全面——后端逐命令强制）。

## 不进 Core 的业务边界

Java 调试 / Redis / 数据库 / MQTT / Kafka / Python 工具等均为插件：
Core 只保留宿主管理能力（生命周期 / 权限 / IPC / 存储隔离 / 通知 / 日志 /
UI 宿主 / Sidecar 监督）。
