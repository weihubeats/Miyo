//! 全局状态管理器：SQLite 连接 + 当前激活 Project / Environment 缓存。
//!
//! - 使用 `tokio::sync::RwLock`（读多写少），Command 并发读取激活上下文；
//! - 激活对象首次访问时从数据库加载并写回缓存，避免重复查询；
//! - `variables_for` 提供「环境 > 项目」合并变量表，供请求渲染使用；
//! - 激活环境按项目记忆（settings 表 `active_environment_id:{project_id}`），
//!   切换项目时各自保持自己的激活环境。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use miyo_core::model::{Environment, GlobalParam, Project};
use miyo_core::VariableMap;
use sqlx::SqlitePool;
use tokio::sync::RwLock as AsyncRwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use miyo_storage::repository as repo;

use crate::error::CommandResult;

/// 激活上下文持久化键（settings 表）。
const KEY_ACTIVE_PROJECT: &str = "active_project_id";
/// 激活环境按项目记忆：键为 `active_environment_id:{project_id}`。
const KEY_ACTIVE_ENVIRONMENT_PREFIX: &str = "active_environment_id:";

/// 某项目的激活环境持久化键。
pub fn active_environment_key(project_id: Uuid) -> String {
    format!("{KEY_ACTIVE_ENVIRONMENT_PREFIX}{project_id}")
}

/// 序列化激活 id 为 settings 值（JSON：`"uuid"` 或 `null`）。
fn setting_value(id: Option<Uuid>) -> String {
    match id {
        Some(id) => serde_json::to_string(&id.to_string()).unwrap_or_else(|_| "null".into()),
        None => "null".into(),
    }
}

/// 读取持久化的激活 id（缺失 / 损坏返回 `None`）。
async fn load_setting_uuid(db: &SqlitePool, key: &str) -> Option<Uuid> {
    repo::get_setting(db, key)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str::<Option<String>>(&v).ok().flatten())
        .and_then(|s| Uuid::parse_str(&s).ok())
}

/// 读取项目持久化的激活环境 id：
/// 缺失 / 损坏返回 `None`；环境不存在或不属于该项目同样按 `None`
/// （环境已随项目删除 / 跨项目数据漂移时不静默恢复错环境）。
async fn load_project_environment_id(db: &SqlitePool, project_id: Uuid) -> Option<Uuid> {
    let id = load_setting_uuid(db, &active_environment_key(project_id)).await?;
    match repo::get_environment(db, id).await {
        Ok(env) if env.project_id == project_id => Some(id),
        _ => None,
    }
}

/// 当前激活上下文（多标签 / 多窗口共享）。
#[derive(Debug, Default)]
pub struct ActiveContext {
    pub project_id: Option<Uuid>,
    /// 缓存的项目（避免重复查询）。
    pub project: Option<Project>,
    pub environment_id: Option<Uuid>,
    /// 缓存的环境。
    pub environment: Option<Environment>,
}

/// 应用全局状态，由插件在 `setup` 中 `app.manage()` 托管。
pub struct AppState {
    pub db: SqlitePool,
    /// 激活上下文（读写并发安全）。
    pub active: AsyncRwLock<ActiveContext>,
    /// 正在运行的 Mock 服务（未启动为 `None`）。
    pub mock: AsyncRwLock<Option<miyo_mock::server::MockServer>>,
    /// 正在运行的 Agent 控制面服务（未启动为 `None`）。
    pub agent: AsyncRwLock<Option<miyo_agent::server::AgentServer>>,
    /// 在途请求的取消令牌注册表（request_id → token；「取消请求」时触发中止）。
    /// 持有期间不 await，普通 `Mutex` 即可。
    pub request_cancels: Mutex<HashMap<String, CancellationToken>>,
    /// 在途长任务的取消令牌注册表（run_id → token；
    /// 压测 `cancel_load_test` 与集合测试 `cancel_test_collection` 共用）。
    pub run_cancels: Mutex<HashMap<String, CancellationToken>>,
    /// WebSocket 会话（connection_id → 会话；含事件转发任务句柄）。
    pub ws: AsyncRwLock<HashMap<String, crate::commands::ws::WsSession>>,
    /// SSE 订阅任务（connection_id → 转发任务句柄；断开即 abort）。
    pub sse: AsyncRwLock<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// 在途 gRPC 服务端流（stream_id → 取消令牌 + 消费任务句柄）。
    /// 与 `request_cancels` 同为普通 Mutex：持有期间不 await。
    pub grpc_streams: Mutex<HashMap<String, (CancellationToken, tokio::task::JoinHandle<()>)>>,
    /// 被 IPC 预览截断（>2MB）的响应全文缓存（request_id → 全文）：
    /// 前端复制 / 保存示例时经 `get_response_body` 按需取回。
    pub response_bodies: Mutex<ResponseBodyCache>,
    /// 全局设置缓存（详见 [`GlobalSettings`]）：`None` = 未构建。
    pub global_settings_cache: RwLock<Option<Arc<GlobalSettings>>>,
    /// 全局设置代数：写路径失效时自增。缓存构建前后比对，
    /// 防止「构建期间恰好发生保存」的旧快照回填（cache-aside 竞态）。
    settings_gen: AtomicU64,
    /// 脚本沙箱（惰性初始化：首个带脚本的请求才启动工作线程 + QuickJS 引擎；
    /// 初始化失败缓存 `None`，后续请求统一报沙箱错误）。
    pub script_sandbox: std::sync::OnceLock<Option<miyo_http::script_sandbox::ScriptSandbox>>,
}

/// 请求热路径的全局设置快照：命中时免去每次请求 2~3 次 settings SELECT +
/// AES-GCM 解密 + JSON 反序列化（压测高并发下随请求数线性放大的固定税）。
/// 写路径（保存全局变量 / 参数、设置超时、备份恢复合并）成功后必须
/// `invalidate_global_settings`，否则后续请求会用到旧值。
#[derive(Debug)]
pub struct GlobalSettings {
    /// 启用项过滤后的全局变量合并表（环境 / 项目变量在其上叠加）。
    pub global_vars: VariableMap,
    pub global_params: Arc<Vec<GlobalParam>>,
    /// 全局请求超时（毫秒）；`None` = 未配置，由调用方兜底内置默认。
    pub http_timeout_ms: Option<u64>,
}

/// 响应全文缓存：容量 4 条 + 10 分钟 TTL，超限 / 过期即弃，
/// 避免大正文（单条可达 20MB）常驻拖垮内存。
#[derive(Default)]
pub struct ResponseBodyCache {
    entries: HashMap<String, (String, std::time::Instant, u64)>,
    next_seq: u64,
}

impl ResponseBodyCache {
    /// 同时保留的最大条数（按插入序号淘汰最旧）。
    const CAPACITY: usize = 4;
    /// 条目有效期：超期视为过期，`get` 返回 `None`。
    const TTL: std::time::Duration = std::time::Duration::from_secs(600);

    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            next_seq: 0,
        }
    }

    fn purge_expired(&mut self) {
        let now = std::time::Instant::now();
        self.entries
            .retain(|_, (_, at, _)| now.duration_since(*at) < Self::TTL);
    }

    pub fn insert(&mut self, key: String, body: String) {
        self.purge_expired();
        while self.entries.len() >= Self::CAPACITY && !self.entries.contains_key(&key) {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, _, seq))| *seq)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    self.entries.remove(&k);
                }
                None => break,
            }
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries
            .insert(key, (body, std::time::Instant::now(), seq));
    }

    pub fn get(&mut self, key: &str) -> Option<String> {
        self.purge_expired();
        self.entries.get(key).map(|(body, _, _)| body.clone())
    }
}

impl AppState {
    pub fn new(db: SqlitePool) -> Self {
        AppState {
            db,
            active: AsyncRwLock::new(ActiveContext::default()),
            mock: AsyncRwLock::new(None),
            agent: AsyncRwLock::new(None),
            request_cancels: Mutex::new(HashMap::new()),
            run_cancels: Mutex::new(HashMap::new()),
            ws: AsyncRwLock::new(HashMap::new()),
            sse: AsyncRwLock::new(HashMap::new()),
            grpc_streams: Mutex::new(HashMap::new()),
            response_bodies: Mutex::new(ResponseBodyCache::new()),
            global_settings_cache: RwLock::new(None),
            settings_gen: AtomicU64::new(0),
            script_sandbox: std::sync::OnceLock::new(),
        }
    }

    /// 脚本沙箱（惰性创建；引擎初始化失败返回错误）。
    pub fn script_sandbox(&self) -> CommandResult<&miyo_http::script_sandbox::ScriptSandbox> {
        self.script_sandbox
            .get_or_init(|| miyo_http::script_sandbox::ScriptSandbox::new().ok())
            .as_ref()
            .ok_or_else(|| crate::error::CommandError::with_code("INTERNAL", "脚本沙箱初始化失败"))
    }

    /// 全局设置快照：缓存命中零查询；未命中查库构建并回填。
    ///
    /// 查库在锁外完成：std RwLock 只包住快照引用读写，不跨 await
    /// （tokio 运行时下持 std 锁跨 await 会阻塞工作线程）。
    /// 构建前后比对代数：期间发生写路径（保存 / 恢复）则放弃回填，
    /// 下一次请求重新构建，避免旧快照长期驻留。
    pub async fn global_settings(&self) -> CommandResult<Arc<GlobalSettings>> {
        if let Some(snap) = self
            .global_settings_cache
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            return Ok(snap);
        }
        let gen_before = self.settings_gen.load(Ordering::Relaxed);
        let (vars, params, timeout) = tokio::join!(
            repo::get_global_variables(&self.db),
            repo::get_global_params(&self.db),
            crate::commands::settings::read_http_timeout_ms(&self.db),
        );
        let snap = Arc::new(GlobalSettings {
            // 先求值再 move key：`(v.key, v.effective_value()…)` 会因 key 先
            // 被 move 触发 E0382（借用了部分 move 的 v）。
            global_vars: vars?
                .into_iter()
                .filter(|v| v.enabled)
                .map(|v| {
                    let value = v.effective_value().to_string();
                    (v.key, value)
                })
                .collect(),
            global_params: Arc::new(params?),
            http_timeout_ms: timeout?,
        });
        if self.settings_gen.load(Ordering::Relaxed) == gen_before {
            *self
                .global_settings_cache
                .write()
                .unwrap_or_else(|p| p.into_inner()) = Some(snap.clone());
        }
        Ok(snap)
    }

    /// 全局设置写路径（保存全局变量 / 参数、设置超时、备份恢复合并）成功后
    /// 必须调用：代数自增 + 清空缓存，下一次请求重新查库构建。
    pub fn invalidate_global_settings(&self) {
        self.settings_gen.fetch_add(1, Ordering::Relaxed);
        *self
            .global_settings_cache
            .write()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// 当前激活项目（缓存命中直接返回；否则查询并写回缓存）。
    pub async fn active_project(&self) -> CommandResult<Option<Project>> {
        loop {
            let read = self.active.read().await;
            if let Some(project) = &read.project {
                return Ok(Some(project.clone()));
            }
            let Some(id) = read.project_id else {
                return Ok(None);
            };
            drop(read);
            let project = repo::get_project(&self.db, id).await?;
            let mut write = self.active.write().await;
            if write.project_id != Some(id) {
                // 查库期间被切换 / 清空：放弃回填，按新上下文重读
                drop(write);
                continue;
            }
            // 回填仅在缓存为空时写入，避免覆盖并发 set_active 刚写入的新值
            if write.project.is_none() {
                write.project = Some(project.clone());
            }
            return Ok(Some(project));
        }
    }

    /// 当前激活环境（缓存命中直接返回；否则查询并写回缓存）。
    ///
    /// 环境为项目维度：上下文中的激活环境即当前激活项目的
    /// 激活环境（切换项目时由 `set_active_project` 换绑）。
    pub async fn active_environment(&self) -> CommandResult<Option<Environment>> {
        loop {
            let read = self.active.read().await;
            if let Some(environment) = &read.environment {
                return Ok(Some(environment.clone()));
            }
            let Some(id) = read.environment_id else {
                return Ok(None);
            };
            drop(read);
            let environment = repo::get_environment(&self.db, id).await?;
            let mut write = self.active.write().await;
            if write.environment_id != Some(id) {
                drop(write);
                continue;
            }
            if write.environment.is_none() {
                write.environment = Some(environment.clone());
            }
            return Ok(Some(environment));
        }
    }

    /// 激活项目被编辑（upsert）后同步覆盖缓存，
    /// 否则 `variables_for` 会继续使用旧变量，直到重启 / 重新激活。
    pub async fn refresh_active_project(&self, project: Project) {
        let mut write = self.active.write().await;
        if write.project_id == Some(project.id) {
            write.project = Some(project);
        }
    }

    /// 激活环境被编辑（upsert）后同步覆盖缓存（同 `refresh_active_project`）。
    pub async fn refresh_active_environment(&self, environment: Environment) {
        let mut write = self.active.write().await;
        if write.environment_id == Some(environment.id) {
            write.environment = Some(environment);
        }
    }

    /// 设置激活项目（`None` 表示清空）。
    ///
    /// 持久化到 settings 表，重启后由 `restore_active` 恢复。
    /// 环境为项目维度：切换项目时激活环境一并换绑为目标项目
    /// 记忆的激活环境（各项目互不影响）。
    /// 数据库查询全部在锁外完成：写锁若跨 await，会阻塞所有并发读
    /// （每次发请求的 `variables_for` 都要读激活上下文）。
    pub async fn set_active_project(&self, project_id: Option<Uuid>) -> CommandResult<()> {
        let project = match project_id {
            Some(id) => Some(repo::get_project(&self.db, id).await?),
            None => None,
        };
        // 目标项目记忆的激活环境（校验归属；无效按无环境处理）
        let environment_id = match project_id {
            Some(pid) => load_project_environment_id(&self.db, pid).await,
            None => None,
        };
        let environment = match environment_id {
            Some(id) => Some(repo::get_environment(&self.db, id).await?),
            None => None,
        };

        let mut write = self.active.write().await;
        write.project_id = project_id;
        write.project = project;
        write.environment_id = environment_id;
        write.environment = environment;
        drop(write);
        repo::set_setting(&self.db, KEY_ACTIVE_PROJECT, &setting_value(project_id)).await?;
        Ok(())
    }

    /// 设置激活环境（`None` 表示不使用环境变量）。持久化到项目键。
    ///
    /// 环境必须属于当前激活项目（哪个项目就编辑哪个项目的环境）；
    /// 未激活项目时拒绝设置。
    pub async fn set_active_environment(&self, environment_id: Option<Uuid>) -> CommandResult<()> {
        let project_id = self
            .active_project()
            .await?
            .map(|p| p.id)
            .ok_or_else(|| miyo_core::validation("请先选择项目再设置激活环境"))?;
        // 查库在锁外：无效 id 在此返回错误，不占用写锁
        let environment = match environment_id {
            Some(id) => {
                let env = repo::get_environment(&self.db, id).await?;
                if env.project_id != project_id {
                    return Err(miyo_core::validation("环境不属于当前项目").into());
                }
                Some(env)
            }
            None => None,
        };
        let mut write = self.active.write().await;
        write.environment_id = environment_id;
        write.environment = environment;
        drop(write);
        repo::set_setting(
            &self.db,
            &active_environment_key(project_id),
            &setting_value(environment_id),
        )
        .await?;
        Ok(())
    }

    /// 启动时恢复持久化的激活项目 / 环境（校验存在性与归属，无效则丢弃）。
    pub async fn restore_active(&self) -> CommandResult<()> {
        let project_id = load_setting_uuid(&self.db, KEY_ACTIVE_PROJECT).await;
        let project_ok = match project_id {
            Some(id) => repo::get_project(&self.db, id).await.is_ok(),
            None => false,
        };
        let project_id = project_id.filter(|_| project_ok);
        let environment_id = match project_id {
            Some(pid) => load_project_environment_id(&self.db, pid).await,
            None => None,
        };
        let mut write = self.active.write().await;
        write.project_id = project_id;
        write.environment_id = environment_id;
        write.project = None;
        write.environment = None;
        Ok(())
    }

    /// 合并变量表：运行时（空）> 环境 > 项目 > 全局。
    ///
    /// 环境侧取「结构化变量（enabled、本地值优先）」扁平表；此外当环境
    /// 配置了 Base URL 时，注入 `base_url = 环境 Base URL`（未显式定义
    /// base_url 变量时），使请求引擎 `{{base_url}}` 拼接继续可用。
    pub async fn variables_for(&self, environment_id: Option<Uuid>) -> CommandResult<VariableMap> {
        let project = self.active_project().await?;
        let environment = match environment_id {
            // 单次请求显式指定环境：临时加载，不改动全局激活状态。
            Some(id) => Some(repo::get_environment(&self.db, id).await?),
            None => self.active_environment().await?,
        };
        // 全局变量走进程内缓存（含启用项过滤），命中时零 DB 往返。
        let global_vars: VariableMap = self.global_settings().await?.global_vars.clone();
        let project_vars = project.map(|p| p.variables).unwrap_or_default();
        let mut environment_vars: VariableMap = environment
            .as_ref()
            .map(|e| e.effective_variables())
            .unwrap_or_default();
        if !environment_vars.contains_key("base_url") {
            if let Some(base) = environment
                .as_ref()
                .map(|e| e.base_url.trim())
                .filter(|b| !b.is_empty())
            {
                environment_vars.insert("base_url".into(), base.to_string());
            }
        }
        // 单次合并（优先级 环境 > 项目 > 全局）：三张表均为 owned，直接 move，
        // 原来两次 `merge_variables` 把全部键值克隆了两遍。
        let mut merged = global_vars;
        for (k, v) in project_vars {
            merged.insert(k, v);
        }
        for (k, v) in environment_vars {
            merged.insert(k, v);
        }
        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{Environment, EnvironmentVariable, Project};
    use std::path::PathBuf;

    /// 响应全文缓存：按插入序淘汰最旧、命中回全文、同键覆盖不淘汰自己。
    #[test]
    fn response_body_cache_evicts_oldest_and_hits() {
        let mut cache = ResponseBodyCache::new();
        for i in 0..ResponseBodyCache::CAPACITY {
            cache.insert(format!("r{i}"), format!("body-{i}"));
        }
        assert_eq!(cache.get("r0").as_deref(), Some("body-0"));
        assert_eq!(cache.get("missing"), None);

        // 同键覆盖：更新内容，不触发淘汰。
        cache.insert("r0".into(), "body-0-v2".into());
        assert_eq!(cache.get("r0").as_deref(), Some("body-0-v2"));
        assert_eq!(cache.get("r1").as_deref(), Some("body-1"));

        // 超容量：插入新键淘汰最旧（r1，seq 最小）。
        cache.insert("r-new".into(), "new".into());
        assert_eq!(cache.get("r1"), None);
        assert_eq!(cache.get("r0").as_deref(), Some("body-0-v2"));
        assert_eq!(cache.get("r-new").as_deref(), Some("new"));
    }

    fn mk_project(name: &str) -> Project {
        Project {
            id: Uuid::new_v4(),
            name: name.into(),
            description: String::new(),
            variables: Default::default(),
            team_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn mk_env(project_id: Uuid, name: &str, base_url: &str) -> Environment {
        Environment {
            id: Uuid::new_v4(),
            project_id,
            name: name.into(),
            base_url: base_url.into(),
            variables: vec![EnvironmentVariable {
                key: "token".into(),
                remote_value: "tok".into(),
                local_value: String::new(),
                enabled: true,
                description: None,
            }],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// {{base_url}} 注入按项目走：每个项目有自己的激活环境，
    /// 切换项目即切到该项目记忆的激活环境基址。
    #[tokio::test]
    async fn base_url_follows_per_project_environment() {
        let path: PathBuf =
            std::env::temp_dir().join(format!("miyo-env-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = miyo_storage::db::init_db(&path).await.expect("建库");
        let state = AppState::new(db.clone());

        let users = mk_project("小奏技术 · 用户服务");
        let open = mk_project("小奏技术 · 开放演示");
        repo::save_project(&db, &users).await.expect("落库项目");
        repo::save_project(&db, &open).await.expect("落库项目");

        let users_env = mk_env(users.id, "开发环境", "http://127.0.0.1:4010");
        let open_env = mk_env(open.id, "开发环境", "https://jsonplaceholder.typicode.com");
        repo::save_environment(&db, &users_env)
            .await
            .expect("落库环境");
        repo::save_environment(&db, &open_env)
            .await
            .expect("落库环境");

        // 激活用户服务 + 其开发环境 → base_url = 用户服务环境基址
        state
            .set_active_project(Some(users.id))
            .await
            .expect("激活项目");
        state
            .set_active_environment(Some(users_env.id))
            .await
            .expect("激活环境");
        let vars = state.variables_for(None).await.expect("变量表");
        assert_eq!(
            vars.get("base_url").map(String::as_str),
            Some("http://127.0.0.1:4010")
        );

        // 切到开放演示（其激活环境未设置 → 无 base_url 注入）
        state.set_active_project(Some(open.id)).await.expect("激活");
        let vars = state.variables_for(None).await.expect("变量表");
        assert_eq!(vars.get("base_url"), None);

        // 设置开放演示的激活环境后 → base_url 跟随
        state
            .set_active_environment(Some(open_env.id))
            .await
            .expect("激活环境");
        let vars = state.variables_for(None).await.expect("变量表");
        assert_eq!(
            vars.get("base_url").map(String::as_str),
            Some("https://jsonplaceholder.typicode.com")
        );

        // 切回用户服务 → 仍是用户服务自己的环境基址（按项目记忆）
        state
            .set_active_project(Some(users.id))
            .await
            .expect("切回");
        let vars = state.variables_for(None).await.expect("变量表");
        assert_eq!(
            vars.get("base_url").map(String::as_str),
            Some("http://127.0.0.1:4010")
        );

        db.close().await;
        let _ = std::fs::remove_file(&path);
    }

    /// 激活环境归属校验：不能把 A 项目的环境设为 B 项目的激活环境。
    #[tokio::test]
    async fn set_active_environment_rejects_foreign_project() {
        let path: PathBuf =
            std::env::temp_dir().join(format!("miyo-own-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = miyo_storage::db::init_db(&path).await.expect("建库");
        let state = AppState::new(db.clone());

        let users = mk_project("用户服务");
        let open = mk_project("开放演示");
        repo::save_project(&db, &users).await.expect("落库项目");
        repo::save_project(&db, &open).await.expect("落库项目");
        let users_env = mk_env(users.id, "开发环境", "http://127.0.0.1:4010");
        repo::save_environment(&db, &users_env)
            .await
            .expect("落库环境");

        state
            .set_active_project(Some(open.id))
            .await
            .expect("激活开放演示");
        // 用户服务的环境不属于开放演示 → 拒绝
        let err = state
            .set_active_environment(Some(users_env.id))
            .await
            .expect_err("跨项目环境应拒绝");
        assert!(err.to_string().contains("不属于当前项目"), "{err}");
        assert_eq!(state.active_environment().await.expect("无激活环境"), None);

        db.close().await;
        let _ = std::fs::remove_file(&path);
    }

    /// 激活项目被编辑后，缓存必须同步刷新：
    /// 否则 `variables_for` 继续用旧变量，直到重启 / 重新激活。
    #[tokio::test]
    async fn refresh_active_project_updates_cache() {
        let path: PathBuf =
            std::env::temp_dir().join(format!("miyo-refresh-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = miyo_storage::db::init_db(&path).await.expect("建库");
        let state = AppState::new(db.clone());

        let mut project = mk_project("项目");
        repo::save_project(&db, &project).await.expect("落库项目");
        state
            .set_active_project(Some(project.id))
            .await
            .expect("激活");
        // 预热缓存
        state.active_project().await.expect("读缓存");

        // 编辑变量（模拟 save_project 的 upsert + 刷新）
        project
            .variables
            .insert("base_url".into(), "http://new".into());
        state.refresh_active_project(project.clone()).await;

        let cached = state.active_project().await.expect("读缓存");
        assert_eq!(
            cached.and_then(|p| p.variables.get("base_url").cloned()),
            Some("http://new".into()),
            "刷新后缓存应含新变量"
        );

        // 非激活项目刷新不生效
        let mut other = project.clone();
        other.id = Uuid::new_v4();
        other
            .variables
            .insert("base_url".into(), "http://other".into());
        state.refresh_active_project(other).await;
        let cached = state.active_project().await.expect("读缓存");
        assert_eq!(
            cached.and_then(|p| p.variables.get("base_url").cloned()),
            Some("http://new".into()),
            "非激活项目不应污染缓存"
        );

        db.close().await;
        let _ = std::fs::remove_file(&path);
    }

    /// 激活项目 / 环境必须跨「重启」恢复：写入 settings 表，重建状态后可读回。
    /// 环境按项目记忆：每个项目各自恢复自己的激活环境。
    #[tokio::test]
    async fn active_context_persists_across_restart() {
        let path: PathBuf =
            std::env::temp_dir().join(format!("miyo-active-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = miyo_storage::db::init_db(&path).await.expect("建库");
        let state = AppState::new(db.clone());

        let project = mk_project("测试项目");
        repo::save_project(&db, &project).await.expect("落库项目");
        let env = mk_env(project.id, "dev", "http://127.0.0.1:4010");
        repo::save_environment(&db, &env).await.expect("落库环境");

        state
            .set_active_project(Some(project.id))
            .await
            .expect("激活项目");
        state
            .set_active_environment(Some(env.id))
            .await
            .expect("激活环境");

        // 模拟重启：同库新建状态，仅靠 settings 表恢复。
        let restarted = AppState::new(db.clone());
        restarted.restore_active().await.expect("恢复激活上下文");
        let read = restarted.active.read().await;
        assert_eq!(read.project_id, Some(project.id), "项目应恢复");
        assert_eq!(read.environment_id, Some(env.id), "环境应恢复");
        drop(read);
        assert_eq!(
            restarted
                .active_environment()
                .await
                .expect("读环境")
                .map(|e| e.id),
            Some(env.id)
        );

        // 回归：重启后用户经项目列表「重新进入」同一项目，环境必须保留
        //（restore 后 environment 缓存为空，旧逻辑会误判归属并清空环境）。
        restarted
            .set_active_project(Some(project.id))
            .await
            .expect("重进项目");
        {
            let read = restarted.active.read().await;
            assert_eq!(
                read.environment_id,
                Some(env.id),
                "重进同一项目不应清空环境"
            );
        }

        // 环境按项目记忆：切换其他项目（无激活环境），当前项目环境保持。
        let other_project_id = Uuid::new_v4();
        repo::save_project(
            &db,
            &Project {
                id: other_project_id,
                ..mk_project("其他项目")
            },
        )
        .await
        .expect("落库其他项目");

        restarted
            .set_active_project(Some(other_project_id))
            .await
            .expect("切换项目");
        {
            let read = restarted.active.read().await;
            assert_eq!(read.project_id, Some(other_project_id));
            assert_eq!(read.environment_id, None, "其他项目无激活环境");
        }

        // 切回原项目 → 恢复原项目记忆的激活环境。
        restarted
            .set_active_project(Some(project.id))
            .await
            .expect("切回原项目");
        {
            let read = restarted.active.read().await;
            assert_eq!(read.environment_id, Some(env.id), "按项目记忆恢复");
        }

        // 重启恢复：环境 id 有效且归属项目即恢复。
        let again = AppState::new(db.clone());
        again.restore_active().await.expect("恢复");
        let read = again.active.read().await;
        assert_eq!(read.project_id, Some(project.id));
        assert_eq!(read.environment_id, Some(env.id), "重启后按项目恢复");
        drop(read);

        db.close().await;
        let _ = std::fs::remove_file(&path);
    }
}
