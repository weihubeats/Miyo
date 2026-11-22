//! JavaBackend：宿主命令层的双路分发（ROADMAP §1 第 3 步）。
//!
//! - [`JavaBackend::Local`]：进程内 [`AttachManager`]（默认路径，行为与
//!   历史版本完全一致）；
//! - [`JavaBackend::Sidecar`]：经 Supervisor 把 req 转发给
//!   `java-debug-sidecar` 二进制——编排（会话表 / AgentClient / 心跳 /
//!   launcher spawn）整体运行在 sidecar 进程内。
//!
//! 模式选择：环境变量 `MIYO_JAVA_SIDECAR_BIN` 指向二进制路径即启用
//! sidecar 分发（缺省 / 路径为空 = Local）。**会话态必须单源**：attach /
//! detach / unload / 查询 / agent 调用全部走同一 backend，禁止混用。
//!
//! data 目录经 [`JavaBackend::Sidecar`] 的 spec extra_env（`MIYO_DATA_DIR`）
//! 注入 sidecar——bin 侧按同优先级推导 jar / infoFile 目录。
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::manager::{AttachManager, AttachOptions};
use crate::model::{AttachSessionSnapshot, JavaProcessInfo, ProcessListSummary};
use miyo_core::AppError;
use miyo_sidecar::{RestartPolicy, SidecarSpec, Supervisor};

/// agent 调用操作（`client()` 长连方法的参数化封装——Local 直调，
/// Sidecar 映射为 `client.*` req）。
#[derive(Debug, Clone)]
pub enum AgentOp {
    Ping,
    Status,
    SpringStatus,
    Classes {
        search: Option<String>,
        include: Option<String>,
        exclude: Option<String>,
        include_jdk: bool,
        limit: u32,
        offset: u32,
    },
    ClassDetail {
        class: String,
    },
    Methods {
        class: String,
    },
    Beans {
        search: String,
        context_id: String,
        limit: u32,
    },
    Invoke {
        params: Value,
        timeout_ms: u64,
    },
    Cancel {
        invocation_id: String,
    },
    Stop,
}

impl AgentOp {
    /// sidecar 侧 method 名（bin dispatch 分支名）。
    fn method(&self) -> &'static str {
        match self {
            AgentOp::Ping => "client.ping",
            AgentOp::Status => "client.status",
            AgentOp::SpringStatus => "client.spring_status",
            AgentOp::Classes { .. } => "client.classes",
            AgentOp::ClassDetail { .. } => "client.class_detail",
            AgentOp::Methods { .. } => "client.methods",
            AgentOp::Beans { .. } => "client.beans",
            AgentOp::Invoke { .. } => "client.invoke",
            AgentOp::Cancel { .. } => "client.cancel",
            AgentOp::Stop => "client.stop",
        }
    }

    fn params(&self) -> Value {
        #[derive(serde::Serialize)]
        struct Classes<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            search: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            include: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            exclude: Option<&'a str>,
            #[serde(rename = "includeJdk")]
            include_jdk: bool,
            limit: u32,
            offset: u32,
        }
        match self {
            AgentOp::Ping | AgentOp::Status | AgentOp::SpringStatus => Value::Null,
            AgentOp::Classes {
                search,
                include,
                exclude,
                include_jdk,
                limit,
                offset,
            } => serde_json::to_value(Classes {
                search: search.as_deref(),
                include: include.as_deref(),
                exclude: exclude.as_deref(),
                include_jdk: *include_jdk,
                limit: *limit,
                offset: *offset,
            })
            .unwrap_or(Value::Null),
            AgentOp::ClassDetail { class } => serde_json::json!({ "class": class }),
            AgentOp::Methods { class } => serde_json::json!({ "class": class }),
            AgentOp::Beans {
                search,
                context_id,
                limit,
            } => serde_json::json!({
                "search": search,
                "contextId": context_id,
                "limit": limit,
            }),
            AgentOp::Invoke { params, timeout_ms } => {
                serde_json::json!({ "params": params, "timeoutMs": timeout_ms })
            }
            AgentOp::Cancel { invocation_id } => {
                serde_json::json!({ "invocationId": invocation_id })
            }
            AgentOp::Stop => Value::Null,
        }
    }

    /// 超时（sidecar 转发等待；invoke 带调用超时余量）。
    fn timeout(&self) -> Duration {
        match self {
            AgentOp::Invoke { timeout_ms, .. } => {
                Duration::from_millis(timeout_ms.saturating_add(20_000))
            }
            _ => Duration::from_secs(30),
        }
    }
}

/// 双路分发后端。
pub enum JavaBackend {
    Local {
        manager: Arc<AttachManager>,
    },
    Sidecar {
        supervisor: Arc<Supervisor>,
        plugin_id: String,
        bin_path: PathBuf,
        data_dir: PathBuf,
    },
}

/// 模式选择的环境变量名（指向 java-debug-sidecar 二进制）。
pub const SIDECAR_BIN_ENV: &str = "MIYO_JAVA_SIDECAR_BIN";

impl JavaBackend {
    /// 解析模式：`MIYO_JAVA_SIDECAR_BIN` 非空 → Sidecar（宿主编排保持默认）。
    pub fn resolve(
        manager: Arc<AttachManager>,
        supervisor: Arc<Supervisor>,
        data_dir: PathBuf,
    ) -> Self {
        match std::env::var(SIDECAR_BIN_ENV) {
            Ok(bin) if !bin.trim().is_empty() => JavaBackend::Sidecar {
                supervisor,
                plugin_id: crate::JAVA_DEBUG_SIDECAR_PLUGIN_ID.into(),
                bin_path: PathBuf::from(bin),
                data_dir,
            },
            _ => JavaBackend::Local { manager },
        }
    }

    /// Sidecar 模式幂等启动（已在运行则零开销；Local 恒 Ok）。
    async fn ensure_started(&self) -> Result<(), AppError> {
        match self {
            JavaBackend::Local { .. } => Ok(()),
            JavaBackend::Sidecar {
                supervisor,
                plugin_id,
                bin_path,
                data_dir,
            } => {
                let running = supervisor
                    .status()
                    .await
                    .iter()
                    .any(|s| s.plugin_id == *plugin_id);
                if running {
                    return Ok(());
                }
                supervisor
                    .start(SidecarSpec {
                        plugin_id: plugin_id.clone(),
                        executable: bin_path.clone(),
                        args: vec![],
                        working_dir: None,
                        max_memory_mb: Some(512),
                        restart_policy: RestartPolicy::OnCrash,
                        // bin 侧按 MIYO_DATA_DIR 推导 jar / infoFile 目录。
                        extra_env: vec![("MIYO_DATA_DIR".into(), data_dir.display().to_string())],
                    })
                    .await
            }
        }
    }

    pub async fn discover_processes(&self) -> Result<ProcessListSummary, AppError> {
        match self {
            JavaBackend::Local { manager } => Ok(manager.discover_processes().await),
            JavaBackend::Sidecar { .. } => {
                self.ensure_started().await?;
                let supervisor = self.supervisor()?;
                let v = supervisor
                    .request(
                        self.plugin_id(),
                        "discover",
                        Value::Null,
                        Duration::from_secs(60),
                    )
                    .await?;
                serde_json::from_value(v)
                    .map_err(|e| AppError::Validation(format!("discover 响应反序列化失败：{e}")))
            }
        }
    }

    pub async fn probe_pid(&self, pid: i64) -> Result<JavaProcessInfo, AppError> {
        match self {
            JavaBackend::Local { manager } => manager.probe_pid(pid).await,
            JavaBackend::Sidecar { .. } => {
                self.ensure_started().await?;
                let supervisor = self.supervisor()?;
                let v = supervisor
                    .request(
                        self.plugin_id(),
                        "probe",
                        serde_json::json!({ "pid": pid }),
                        Duration::from_secs(30),
                    )
                    .await?;
                serde_json::from_value(v)
                    .map_err(|e| AppError::Validation(format!("probe 响应反序列化失败：{e}")))
            }
        }
    }

    pub async fn attach(
        &self,
        pid: i64,
        options: &AttachOptions,
    ) -> Result<AttachSessionSnapshot, AppError> {
        match self {
            JavaBackend::Local { manager } => manager.attach(pid, options).await,
            JavaBackend::Sidecar { .. } => {
                self.ensure_started().await?;
                let supervisor = self.supervisor()?;
                let v = supervisor
                    .request(
                        self.plugin_id(),
                        "attach",
                        serde_json::json!({
                            "pid": pid,
                            "options": serde_json::to_value(options)
                                .map_err(|e| AppError::Connection(e.to_string()))?,
                        }),
                        Duration::from_secs(40),
                    )
                    .await?;
                serde_json::from_value(v)
                    .map_err(|e| AppError::Validation(format!("attach 响应反序列化失败：{e}")))
            }
        }
    }

    pub async fn detach(&self, pid: i64) -> Result<(), AppError> {
        match self {
            JavaBackend::Local { manager } => manager.detach(pid).await,
            JavaBackend::Sidecar { .. } => self.request_null("detach", pid).await,
        }
    }

    pub async fn unload_agent(&self, pid: i64) -> Result<(), AppError> {
        match self {
            JavaBackend::Local { manager } => manager.unload_agent(pid).await,
            JavaBackend::Sidecar { .. } => self.request_null("unload", pid).await,
        }
    }

    pub async fn all_snapshots(&self) -> Result<Vec<AttachSessionSnapshot>, AppError> {
        match self {
            JavaBackend::Local { manager } => Ok(manager.all_snapshots().await),
            JavaBackend::Sidecar { .. } => {
                self.ensure_started().await?;
                let supervisor = self.supervisor()?;
                let v = supervisor
                    .request(
                        self.plugin_id(),
                        "sessions",
                        Value::Null,
                        Duration::from_secs(30),
                    )
                    .await?;
                serde_json::from_value(v)
                    .map_err(|e| AppError::Validation(format!("sessions 响应反序列化失败：{e}")))
            }
        }
    }

    /// agent 调用（classes / methods / beans / invoke / cancel …）。
    pub async fn agent_call(&self, pid: i64, op: AgentOp) -> Result<Value, AppError> {
        match self {
            JavaBackend::Local { manager } => {
                let client = manager.client(pid).await?;
                local_client_call(&client, op).await
            }
            JavaBackend::Sidecar { .. } => {
                self.ensure_started().await?;
                let supervisor = self.supervisor()?;
                let mut params = op.params();
                if let Value::Null = params {
                    params = Value::Object(serde_json::Map::new());
                }
                if let Value::Object(ref mut map) = params {
                    map.insert("pid".into(), Value::from(pid));
                }
                supervisor
                    .request(self.plugin_id(), op.method(), params, op.timeout())
                    .await
            }
        }
    }

    fn supervisor(&self) -> Result<&Arc<Supervisor>, AppError> {
        match self {
            JavaBackend::Local { .. } => {
                Err(AppError::Connection("Local 模式无 Supervisor".into()))
            }
            JavaBackend::Sidecar { supervisor, .. } => Ok(supervisor),
        }
    }

    fn plugin_id(&self) -> &str {
        match self {
            JavaBackend::Local { .. } => "",
            JavaBackend::Sidecar { plugin_id, .. } => plugin_id,
        }
    }

    /// null 结果类方法（detach / unload：sidecar 返回无 body）。
    async fn request_null(&self, method: &str, pid: i64) -> Result<(), AppError> {
        self.ensure_started().await?;
        let supervisor = self.supervisor()?;
        supervisor
            .request(
                self.plugin_id(),
                method,
                serde_json::json!({ "pid": pid }),
                Duration::from_secs(30),
            )
            .await?;
        Ok(())
    }
}

/// Local 直调分发（AgentClient 方法面）。
async fn local_client_call(
    client: &crate::client::AgentClient,
    op: AgentOp,
) -> Result<Value, AppError> {
    match op {
        AgentOp::Ping => client.ping().await.map_err(miyo_core::AppError::from),
        AgentOp::Status => client.status().await.map_err(miyo_core::AppError::from),
        AgentOp::SpringStatus => client
            .spring_status()
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::Classes {
            search,
            include,
            exclude,
            include_jdk,
            limit,
            offset,
        } => client
            .classes(
                search.as_deref(),
                include.as_deref(),
                exclude.as_deref(),
                include_jdk,
                limit,
                offset,
            )
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::ClassDetail { class } => client
            .class_detail(&class)
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::Methods { class } => client
            .methods(&class)
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::Beans {
            search,
            context_id,
            limit,
        } => client
            .beans(&search, &context_id, limit)
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::Invoke { params, timeout_ms } => client
            .invoke(params, timeout_ms)
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::Cancel { invocation_id } => client
            .cancel(&invocation_id)
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::Stop => client.stop_agent().await.map_err(miyo_core::AppError::from),
    }
}
