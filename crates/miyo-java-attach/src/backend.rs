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
    Classes(ClassesQuery),
    /// 类分类统计（业务 / Bean / 依赖 / JDK / 未识别计数 + 启动信息证据）。
    ClassesStats {
        business_packages: Option<String>,
    },
    /// 全量重建类索引（业务包配置变化后）。
    ClassesReindex {
        business_packages: Option<String>,
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

/// 类清单查询参数（`classes.list`）。
///
/// 分类维度（与 Java agent 的 `ClassInspector.ListQuery` 一一对应）：
/// category / package_prefix / include_dependencies / include_jdk，兼容老字段
/// include / exclude。全部字段可选——缺省即 agent 侧默认（Spring Bean + 业务类）。
#[derive(Debug, Clone, Default)]
pub struct ClassesQuery {
    pub search: Option<String>,
    pub include: Option<String>,
    pub exclude: Option<String>,
    /// 类别筛选（wire 名：application / spring_bean / dependency / jdk_internal /
    /// generated / unknown），逗号分隔。
    pub category: Option<String>,
    /// 包名前缀（`com.example,com.foo.*`）。
    pub package_prefix: Option<String>,
    pub include_dependencies: bool,
    pub include_jdk: bool,
    pub limit: u32,
    pub offset: u32,
    /// 用户配置的业务包前缀（分类弱证据；由前端持久化后透传）。
    pub business_packages: Option<String>,
    /// 是否刷新索引（默认 true = 增量；false 只读缓存）。
    pub refresh_index: bool,
}

impl ClassesQuery {
    /// 前端只传 pid 时的默认查询（业务类 + Spring Bean，首页 200 条）。
    pub fn new() -> Self {
        Self {
            limit: 200,
            refresh_index: true,
            ..Self::default()
        }
    }

    pub(crate) fn to_params(&self) -> Value {
        let mut map = serde_json::Map::new();
        let mut put = |key: &str, value: Option<&str>| {
            if let Some(text) = value {
                map.insert(key.to_string(), Value::String(text.to_string()));
            }
        };
        put("search", self.search.as_deref());
        put("include", self.include.as_deref());
        put("exclude", self.exclude.as_deref());
        put("category", self.category.as_deref());
        put("package_prefix", self.package_prefix.as_deref());
        put("business_packages", self.business_packages.as_deref());
        map.insert(
            "include_dependencies".into(),
            Value::Bool(self.include_dependencies),
        );
        map.insert("include_jdk".into(), Value::Bool(self.include_jdk));
        map.insert("refresh_index".into(), Value::Bool(self.refresh_index));
        map.insert("limit".into(), Value::from(self.limit));
        map.insert("offset".into(), Value::from(self.offset));
        Value::Object(map)
    }
}

impl AgentOp {
    /// sidecar 侧 method 名（bin dispatch 分支名）。
    fn method(&self) -> &'static str {
        match self {
            AgentOp::Ping => "client.ping",
            AgentOp::Status => "client.status",
            AgentOp::SpringStatus => "client.spring_status",
            AgentOp::Classes(_) => "client.classes",
            AgentOp::ClassesStats { .. } => "client.classes_stats",
            AgentOp::ClassesReindex { .. } => "client.classes_reindex",
            AgentOp::ClassDetail { .. } => "client.class_detail",
            AgentOp::Methods { .. } => "client.methods",
            AgentOp::Beans { .. } => "client.beans",
            AgentOp::Invoke { .. } => "client.invoke",
            AgentOp::Cancel { .. } => "client.cancel",
            AgentOp::Stop => "client.stop",
        }
    }

    fn params(&self) -> Value {
        match self {
            AgentOp::Ping | AgentOp::Status | AgentOp::SpringStatus => Value::Null,
            AgentOp::Classes(query) => query.to_params(),
            AgentOp::ClassesStats { business_packages } => serde_json::json!({
                "package_prefix": business_packages,
            }),
            AgentOp::ClassesReindex { business_packages } => serde_json::json!({
                "package_prefix": business_packages,
            }),
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
        AgentOp::Classes(query) => client
            .classes(&query)
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::ClassesStats { business_packages } => client
            .classes_stats(business_packages.as_deref())
            .await
            .map_err(miyo_core::AppError::from),
        AgentOp::ClassesReindex { business_packages } => client
            .classes_reindex(business_packages.as_deref())
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
