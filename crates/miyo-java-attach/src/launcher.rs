//! Launcher sidecar 运行器：以 `java -jar miyo-attach-launcher.jar <cmd>` 方式
//! 调用 Attach CLI（短进程），解析单行 JSON 输出。
//!
//! 进程隔离的意义：Attach API 的异常路径（AttachNotSupportedException /
//! AgentLoadException…）不会污染 Miyo 主进程；launcher 崩溃仅体现为一次
//! 结构化错误。

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;

use miyo_core::AppError;

use crate::jdk::JdkToolchain;

/// launcher / agent jar 路径组。
#[derive(Debug, Clone)]
pub struct LauncherToolPaths {
    pub launcher_jar: PathBuf,
    pub agent_jar: PathBuf,
}

impl LauncherToolPaths {
    /// 就绪检查（前端诊断用）。
    pub fn ready(&self) -> bool {
        self.launcher_jar.is_file() && self.agent_jar.is_file()
    }
}

/// launcher `capabilities` 输出。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherCapabilities {
    pub java_version: String,
    pub java_home: String,
    #[serde(rename = "os")]
    pub os: String,
    pub os_arch: String,
    pub bridge_available: bool,
    pub bridge_desc: String,
    pub tools_jar: Option<String>,
}

/// launcher `list` 输出条目。
#[derive(Debug, Clone, Deserialize)]
pub struct LauncherVm {
    pub pid: i64,
    pub display_name: String,
}

/// launcher `attach` 成功输出（agent 写入的 infoFile 内容）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AttachInfoFile {
    pub ok: bool,
    pub protocol: u32,
    pub agent_version: String,
    pub pid: i64,
    pub port: u16,
    pub vm_start_ms: i64,
    pub started_at: i64,
    pub written_at: i64,
    #[serde(default)]
    pub error: Option<String>,
    /// 重复 attach 命中（Agent 已在目标 JVM 运行，本次仅换发 token）。
    #[serde(default)]
    pub existing: bool,
}

/// launcher 命令执行器。
pub struct LauncherRunner {
    /// launcher 使用的 java 可执行文件（工具链选择结果）。
    java_executable: PathBuf,
    pub tools: LauncherToolPaths,
}

impl LauncherRunner {
    pub fn new(toolchain: &JdkToolchain, tools: LauncherToolPaths) -> Self {
        LauncherRunner {
            java_executable: toolchain.java_executable(),
            tools,
        }
    }

    /// `capabilities`：工具链自检（bridge_available=false = 该 JDK 不能 attach）。
    pub async fn capabilities(&self) -> Result<LauncherCapabilities, AppError> {
        let raw = self.run("capabilities", &[], 15_000, None).await?;
        parse_json(&raw)
    }

    /// `list`：目标 JVM 里 attach listener 可达的进程（pid + display_name）。
    pub async fn list_attachable(&self) -> Result<Vec<LauncherVm>, AppError> {
        let raw = self.run("list", &[], 15_000, None).await?;
        let parsed: serde_json::Value = parse_json(&raw)?;
        let items = parsed
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|item| {
                let pid = item.get("pid")?.as_i64()?;
                let display_name = item
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                Some(LauncherVm { pid, display_name })
            })
            .collect())
    }

    /// `properties`：读目标 JVM 系统属性（预检版本 / 厂商，不加载 Agent）。
    pub async fn properties(
        &self,
        pid: i64,
    ) -> Result<std::collections::HashMap<String, String>, AppError> {
        let raw = self
            .run(
                "properties",
                &["--pid".into(), pid.to_string()],
                20_000,
                None,
            )
            .await?;
        let parsed: serde_json::Value = parse_json(&raw)?;
        let mut out = std::collections::HashMap::new();
        if let Some(obj) = parsed.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    out.insert(k.clone(), s.to_string());
                }
            }
        }
        Ok(out)
    }

    /// `attach`：attach + loadAgent + 轮询 infoFile。成功返回连接信息。
    ///
    /// 安全：`agent_args`（含一次性 token）经 **stdin** 传给 launcher
    /// （`--args-stdin`），不出现在子进程命令行——避免 attach 窗口期内本机
    /// 其他用户经 `ps` 读到 token。Java 侧仍兼容 `--args`（旧调用方）。
    pub async fn attach(
        &self,
        pid: i64,
        agent_args: &str,
        info_file: &PathBuf,
        timeout_ms: u64,
    ) -> Result<AttachInfoFile, AppError> {
        // infoFile 先删（幂等起点：陈旧文件会让轮询读到上一轮端口）
        tokio::fs::remove_file(info_file).await.ok();
        let raw = self
            .run(
                "attach",
                &[
                    "--pid".into(),
                    pid.to_string(),
                    "--agent".into(),
                    self.tools.agent_jar.to_string_lossy().to_string(),
                    "--args-stdin".into(),
                    "--info-file".into(),
                    info_file.to_string_lossy().to_string(),
                    "--timeout-ms".into(),
                    timeout_ms.to_string(),
                ],
                timeout_ms + 30_000, // launcher 自身 JVM 启动余量
                Some(agent_args),
            )
            .await?;
        let info: AttachInfoFile = parse_json(&raw)?;
        if !info.ok {
            return Err(AppError::Connection(format!(
                "Agent 初始化失败：{}",
                info.error.unwrap_or_else(|| "（无详情）".into())
            )));
        }
        Ok(info)
    }

    async fn run(
        &self,
        command: &str,
        args: &[String],
        timeout_ms: u64,
        stdin_data: Option<&str>,
    ) -> Result<String, AppError> {
        if !self.tools.ready() {
            return Err(AppError::NotFound(format!(
                "Attach 工具 jar 缺失（launcher={}，agent={}）。请运行 scripts/build_java_debug.sh 构建，\
                 或重新安装 Miyo（安装包内置）",
                self.tools.launcher_jar.display(),
                self.tools.agent_jar.display()
            )));
        }
        let mut cmd = Command::new(&self.java_executable);
        cmd.arg("-jar")
            .arg(&self.tools.launcher_jar)
            .arg(command)
            .args(args)
            .stdin(if stdin_data.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(AppError::Io)?;
        // stdin 载荷（attach 的 agent_args）：写入后立刻 drop 关闭写端，
        // launcher 读到 EOF 再继续。write_all 阻塞上限为子进程消费速度，
        // 外层 timeout 覆盖整个 spawn→写→等退出链路。
        if let (Some(data), Some(mut stdin)) = (stdin_data, child.stdin.take()) {
            use tokio::io::AsyncWriteExt;
            if let Err(e) = stdin.write_all(data.as_bytes()).await {
                tracing::warn!("launcher stdin 写入失败（子进程可能已退出）：{e}");
            }
            let _ = stdin.shutdown().await;
            drop(stdin);
        }
        let output =
            tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output())
                .await
                .map_err(|_| {
                    AppError::NetworkTimeout(format!(
                        "launcher {command} 超时（{timeout_ms}ms；java={}）",
                        self.java_executable.display()
                    ))
                })?
                .map_err(AppError::Io)?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            // launcher 失败时 stdout 仍是结构化错误 JSON（错误码 → AppError 语义）
            if !stdout.is_empty() {
                return Err(launcher_error(&stdout, &stderr));
            }
            return Err(AppError::Connection(format!(
                "launcher {command} 退出码 {}：{}",
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() {
                    "（无输出）".into()
                } else {
                    stderr
                }
            )));
        }
        if stdout.is_empty() {
            return Err(AppError::Connection(format!(
                "launcher {command} 无输出（stderr: {stderr}）"
            )));
        }
        Ok(stdout)
    }
}

/// launcher 错误 JSON → AppError（错误码分型：attach_not_supported /
/// permission_denied / process_not_found / no_attach_toolchain / …）。
fn launcher_error(stdout: &str, stderr: &str) -> AppError {
    #[derive(Deserialize)]
    struct LauncherError {
        code: String,
        message: String,
        #[serde(default)]
        hint: Option<String>,
    }
    #[derive(Deserialize)]
    struct ErrorBody {
        #[serde(default)]
        error: Option<LauncherError>,
    }
    if let Ok(body) = serde_json::from_str::<ErrorBody>(stdout) {
        if let Some(err) = body.error {
            let detail = match err.hint {
                Some(hint) => format!("{}（{}）", err.message, hint),
                None => err.message,
            };
            return match err.code.as_str() {
                "attach_not_supported" => {
                    AppError::Forbidden(format!("目标 JVM 不支持 attach：{detail}"))
                }
                "permission_denied" => AppError::Forbidden(format!("attach 权限不足：{detail}")),
                "process_not_found" => AppError::NotFound(format!("进程不存在：{detail}")),
                "no_attach_toolchain" => AppError::NotFound(format!("无 Attach 工具链：{detail}")),
                "agent_load_failed" | "agent_init_failed" => {
                    AppError::Connection(format!("Agent 加载失败：{detail}"))
                }
                "timeout" => AppError::NetworkTimeout(format!("attach 超时：{detail}")),
                _ => AppError::Connection(format!("launcher 失败：{detail}")),
            };
        }
    }
    // 非结构化输出：原样带出
    AppError::Connection(format!(
        "launcher 失败：{} {}",
        stdout.chars().take(200).collect::<String>(),
        stderr.chars().take(200).collect::<String>()
    ))
}

fn parse_json<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, AppError> {
    serde_json::from_str(raw).map_err(|e| {
        AppError::Connection(format!(
            "launcher 输出不是合法 JSON：{e}（原文前 200 字：{}）",
            raw.chars().take(200).collect::<String>()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_error_codes_map() {
        let raw = r#"{"error":{"code":"attach_not_supported","message":"目标 JVM 不支持 attach","hint":"DisableAttachMechanism"}}"#;
        let err = launcher_error(raw, "");
        assert!(matches!(err, AppError::Forbidden(_)), "{err:?}");
        let raw = r#"{"error":{"code":"process_not_found","message":"No such process"}}"#;
        let err = launcher_error(raw, "");
        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
        let err = launcher_error("garbage output", "stack");
        assert!(matches!(err, AppError::Connection(_)), "{err:?}");
    }

    #[test]
    fn attach_args_json_shape() {
        // AttachInfoFile 解析对齐 Java 侧 infoFile 字段（snake_case）
        let raw = r#"{"ok":true,"protocol":2,"agent_version":"2.0.0","pid":123,"port":54321,
            "vm_start_ms":1234,"started_at":1234,"written_at":1235}"#;
        let info: AttachInfoFile = serde_json::from_str(raw).unwrap();
        assert_eq!(info.port, 54321);
        assert_eq!(info.protocol, 2);
        assert!(info.error.is_none());
    }
}
