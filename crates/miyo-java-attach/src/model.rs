//! 数据模型（前端 IPC 形状，camelCase serde）。
//!
//! `JavaProcessInfo` 是发现层输出；`AttachSessionSnapshot` 是会话管理层
//! 回传前端的快照（token 永不出 Miyo 进程——快照不含 token）。

use serde::{Deserialize, Serialize};

/// 本机 Java 进程（发现层输出）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JavaProcessInfo {
    pub pid: i64,
    /// 主类或 -jar 入口（如 `com.example.PlainApp` / `app.jar`；解析不出为空串）。
    pub main_class: String,
    /// 展示名（main_class 或完整命令行截断）。
    pub display_name: String,
    /// 完整命令行（敏感参数已脱敏）。
    pub command_line: String,
    /// Java 可执行文件路径。
    pub java_executable: String,
    /// 推断的 JAVA_HOME（`<jdk>/Contents/Home` 或 `<jdk>`；解析不出为空）。
    pub java_home: String,
    /// Java 版本（`<home>/bin/java -version` 缓存；未探测为空）。
    pub java_version: String,
    /// JVM 厂商（同上缓存）。
    pub jvm_vendor: String,
    /// jdk / jre / unknown（javac 存在性判定）。
    pub kind: String,
    /// 启动时刻（`ps lstart` / /proc；RFC3339 或原始文本）。
    pub started_at: String,
    /// 运行时长（etime 原始文本，如 `03:27`）。
    pub elapsed: String,
    /// 用户 id / 用户名（区分「他人进程不可 attach」）。
    pub uid: String,
    /// launcher list 命中 → attachable；否则按 uid / 平台给 reason。
    pub attachable: bool,
    pub attach_block_reason: Option<String>,
    /// 当前已 attach（本 Miyo 会话内）。
    pub attached: bool,
    /// 来源标记：`os_scan` / `manual_pid`。
    pub source: String,
}

impl Default for JavaProcessInfo {
    fn default() -> Self {
        JavaProcessInfo {
            pid: 0,
            main_class: String::new(),
            display_name: String::new(),
            command_line: String::new(),
            java_executable: String::new(),
            java_home: String::new(),
            java_version: String::new(),
            jvm_vendor: String::new(),
            kind: "unknown".into(),
            started_at: String::new(),
            elapsed: String::new(),
            uid: String::new(),
            attachable: false,
            attach_block_reason: None,
            attached: false,
            source: "os_scan".into(),
        }
    }
}

/// 发现汇总（含工具链可用性，前端据此提示诊断）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessListSummary {
    pub processes: Vec<JavaProcessInfo>,
    /// 本机 JDK 工具链数量（0 = 无可运行 launcher 的 JDK，前端引导安装）。
    pub toolchain_count: usize,
    /// launcher / agent jar 就绪。
    pub launcher_ready: bool,
    pub agent_ready: bool,
    /// 扫描耗时（毫秒）。
    pub scan_ms: u64,
    /// 平台提示（容器 / 权限等）。
    pub hints: Vec<String>,
}

/// Agent 握手信息（agent → Miyo，`handshake` 响应 data）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", default)]
pub struct AgentHandshakeInfo {
    pub protocol: u32,
    pub agent_version: String,
    pub pid: i64,
    pub vm_start_ms: i64,
    pub java_version: String,
    pub java_vm_version: String,
    pub jvm_vendor: String,
    pub java_home: String,
    pub max_concurrent: u32,
    pub max_timeout_ms: u64,
    pub uptime_ms: u64,
}

impl Default for AgentHandshakeInfo {
    fn default() -> Self {
        AgentHandshakeInfo {
            protocol: 0,
            agent_version: String::new(),
            pid: 0,
            vm_start_ms: 0,
            java_version: String::new(),
            java_vm_version: String::new(),
            jvm_vendor: String::new(),
            java_home: String::new(),
            max_concurrent: 4,
            max_timeout_ms: 300_000,
            uptime_ms: 0,
        }
    }
}

/// attach 成功后的会话快照（回传前端；不含 token）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AttachSessionSnapshot {
    pub pid: i64,
    pub vm_start_ms: i64,
    pub agent_version: String,
    pub protocol: u32,
    /// 目标 JVM 信息（握手回传快照）。
    pub handshake: AgentHandshakeInfo,
    /// attach 时选用的 launcher JDK。
    pub launcher_java_home: String,
    pub attached_at: String,
    /// 重复 attach 检测命中（Agent 已在目标 JVM 运行，本次仅换发 token）。
    pub reattached: bool,
}

/// 启动参数敏感值脱敏（发现层输出前统一调用）。
///
/// 规则：`-Dxxx.password=…` / `--xxx.password=…` / token / secret / key 等
/// 键名命中的值替换为 `***`；密码型 URL（`user:pass@`）的 pass 段脱敏。
pub fn redact_command_line(cmd: &str) -> String {
    const SENSITIVE_KEYS: &[&str] = &[
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "credential",
        "api_key",
        "apikey",
        "access_key",
        "secret_key",
        "private_key",
        "auth",
    ];
    let mut out = Vec::new();
    for token in cmd.split_whitespace() {
        out.push(redact_token(token, SENSITIVE_KEYS));
    }
    out.join(" ")
}

fn redact_token(token: &str, keys: &[&str]) -> String {
    // key=value 形态：键名（去 - / D 前缀，归一小写）以敏感词结尾即脱敏。
    // 覆盖 -Dspring.datasource.password / --api-key / auth= 等常见形态。
    if let Some(eq) = token.find('=') {
        let key = &token[..eq];
        // 去前缀：-- / -（JVM -D 的 D 也去——-Dspring.datasource.password）
        let norm = key
            .strip_prefix("--")
            .or_else(|| key.strip_prefix('-'))
            .unwrap_or(key)
            .trim_start_matches('D')
            .to_lowercase()
            .replace('-', "_");
        for k in keys {
            if norm.ends_with(k) {
                return format!("{key}=***");
            }
        }
    }
    // URL 内嵌凭证 user:pass@host → user:***@host
    if let Some(at) = token.rfind('@') {
        if let Some(scheme) = token.find("://") {
            let user_part = &token[scheme + 3..at];
            if user_part.contains(':') {
                let user = user_part.split(':').next().unwrap_or("");
                return format!("{}{}:***{}", &token[..scheme + 3], user, &token[at..]);
            }
        }
    }
    token.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_masks_password_system_props() {
        let cmd =
            "java -Dspring.datasource.password=pg123 -jar app.jar --api-key=abc --server.port=8080";
        let out = redact_command_line(cmd);
        assert!(out.contains("-Dspring.datasource.password=***"), "{out}");
        assert!(out.contains("--api-key=***"), "{out}");
        assert!(out.contains("--server.port=8080"), "{out}");
        assert!(!out.contains("pg123"));
        assert!(!out.contains("abc"), "api-key 值必须脱敏: {out}");
    }

    #[test]
    fn redact_masks_url_credentials() {
        let out = redact_command_line("java -jar app.jar --url=postgres://admin:secret@db:5432/x");
        assert!(out.contains("postgres://admin:***@db:5432/x"), "{out}");
        assert!(!out.contains("secret"));
    }

    #[test]
    fn redact_keeps_normal_args() {
        let cmd = "java -Xmx512m -cp build com.example.PlainApp --mode=dev";
        assert_eq!(redact_command_line(cmd), cmd);
    }
}
