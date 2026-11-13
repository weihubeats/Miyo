//! 团队同步：项目快照经远端（git / S3 / db）推拉，实现跨机器、跨同事数据一致。
//!
//! 核心语义：本地 SQLite 仍是唯一真源，同步是显式快照动作而非实时协同。
//! - `push`：导出项目快照（miyo-backup v4）→ transport 写入远端 → 记录 revision
//! - `pull`：transport 读远端快照 → 上层预览/diff → 覆盖恢复（先自动快照兜底）

use async_trait::async_trait;
use miyo_backup::BackupFile;
use serde::{Deserialize, Serialize};

pub mod engine;
pub mod git;
pub mod redact;

#[cfg(test)]
pub(crate) mod tests_support {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use miyo_backup::BackupFile;
    use miyo_core::model::{Endpoint, EndpointStatus, HttpMethod, Project, RequestSpec};

    use crate::{RemoteTransport, SnapshotDiff, SyncMessage, TransportError};

    /// 内存 transport：单键快照仓库，引擎测试用。
    #[derive(Default)]
    pub struct MemoryTransport {
        pub store: Mutex<std::collections::HashMap<String, (BackupFile, String)>>,
        pub key: String,
    }

    #[async_trait]
    impl RemoteTransport for MemoryTransport {
        async fn push(&self, s: &BackupFile, _m: &SyncMessage) -> Result<String, TransportError> {
            let mut store = self.store.lock().unwrap();
            let rev = format!("rev-{}", store.len());
            store.insert(self.key.clone(), (s.clone(), rev.clone()));
            Ok(rev)
        }
        async fn pull(&self) -> Result<(BackupFile, String), TransportError> {
            self.store
                .lock()
                .unwrap()
                .get(&self.key)
                .cloned()
                .ok_or_else(|| TransportError::Remote("空远端".into()))
        }
        async fn diff(&self, since: Option<&str>) -> Result<SnapshotDiff, TransportError> {
            let store = self.store.lock().unwrap();
            let Some((head, _)) = store.get(&self.key) else {
                // 空远端：无差异。
                return Ok(SnapshotDiff::default());
            };
            // 内存 transport 无 revision 历史：since 一律视为无旧快照。
            let _ = since;
            Ok(crate::diff_snapshots(None, head))
        }
    }

    /// 构造只含一个接口的最小快照（git 传输端到端测试用）。
    pub fn sample_snapshot(path: &str) -> BackupFile {
        let project = Project {
            id: uuid::Uuid::new_v4(),
            name: "同步测试项目".into(),
            description: String::new(),
            variables: Default::default(),
            team_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        BackupFile {
            format: miyo_backup::FORMAT.to_string(),
            schema_version: miyo_backup::SCHEMA_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            project,
            folders: vec![],
            endpoints: vec![Endpoint {
                id: uuid::Uuid::new_v4(),
                project_id: uuid::Uuid::new_v4(),
                folder_id: None,
                name: path.to_string(),
                method: HttpMethod::GET,
                path: path.to_string(),
                description: String::new(),
                status: EndpointStatus::Released,
                sort_order: 0,
                request: RequestSpec::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }],
            environments: vec![],
            mock_rules: vec![],
            response_examples: vec![],
            request_examples: vec![],
            proto_files: vec![],
            settings: Default::default(),
            global_variables: vec![],
            global_params: vec![],
            team: None,
            scenarios: vec![],
        }
    }
}

/// 同步远端类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteKind {
    Git,
    S3,
    Db,
}

/// 远端配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteConfig {
    /// git 仓库：url + 分支 + 快照文件所在路径。
    Git {
        url: String,
        #[serde(default = "default_branch")]
        branch: String,
        /// 快照在仓库内的路径（目录形态，多项目各一个文件）。
        #[serde(default)]
        path: String,
    },
    S3 {
        bucket: String,
        prefix: String,
    },
    Db {
        dsn_ref: String,
        table: String,
    },
}

fn default_branch() -> String {
    "main".to_string()
}

/// 远端上的一个版本（git commit / S3 object version / DB 行 id）。
pub type Revision = String;

/// 推送说明（git commit message 等）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMessage {
    pub summary: String,
}

/// 快照差异（远端最新 vs 上次同步）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct SnapshotDiff {
    /// 新增的接口（远端有、本地旧 revision 无）。
    pub added: Vec<String>,
    /// 删除的接口（本地旧 revision 有、远端无）。
    pub removed: Vec<String>,
    /// 修改的接口（method+path 相同但内容不同）。
    pub modified: Vec<String>,
}

impl SnapshotDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }
}

/// 传输错误（网络 / 凭证 / 远端不可达），与快照语义错误分离。
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("网络 / 远端错误：{0}")]
    Remote(String),
    #[error("凭证错误：{0}")]
    Credential(String),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("远端快照 schema 过新（远端 {remote} > 本地 {local}），请升级客户端")]
    SchemaMismatch { remote: u32, local: u32 },
}

impl From<TransportError> for miyo_core::AppError {
    fn from(e: TransportError) -> Self {
        // 专属 SYNC 错误码（原借道 VALIDATION）：远端传输失败可被前端按码识别，
        // 与「请求参数非法」区分开。
        miyo_core::AppError::Sync(e.to_string())
    }
}

/// 远端传输抽象：三种媒介（git/S3/db）统一为快照推拉语义。
#[async_trait]
pub trait RemoteTransport: Send + Sync {
    /// 推送快照到远端，返回新 revision。
    async fn push(
        &self,
        snapshot: &BackupFile,
        msg: &SyncMessage,
    ) -> Result<Revision, TransportError>;

    /// 拉取远端最新快照。
    async fn pull(&self) -> Result<(BackupFile, Revision), TransportError>;

    /// 比较远端最新与指定 revision 的快照差异（接口粒度）。
    async fn diff(&self, since: Option<&str>) -> Result<SnapshotDiff, TransportError>;

    /// 列出远端的项目快照清单（导入入口：发现远端新增项目）。
    /// 默认不支持（S3/db 未实现），git 覆写。
    async fn list_projects(&self) -> Result<Vec<RemoteProjectEntry>, TransportError> {
        Err(TransportError::Remote("该远端类型暂不支持列出项目".into()))
    }
}

/// 远端项目快照条目（列表展示 + 导入判定用）。
#[derive(Debug, Clone, Serialize)]
pub struct RemoteProjectEntry {
    pub project_id: String,
    pub name: String,
    pub endpoint_count: usize,
    pub updated_at: String,
}

/// 接口快照的粒度键（method + path）。
pub(crate) fn endpoint_key(e: &miyo_core::model::Endpoint) -> String {
    format!("{} {}", e.method, e.path)
}

/// 基于两个快照计算接口粒度差异（transport 无关的公共实现）。
pub fn diff_snapshots(old: Option<&BackupFile>, new: &BackupFile) -> SnapshotDiff {
    use std::collections::HashMap;
    let old_map: HashMap<String, String> = old
        .map(|f| {
            f.endpoints
                .iter()
                .map(|e| {
                    (
                        endpoint_key(e),
                        serde_json::to_string(e).unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let new_map: HashMap<String, String> = new
        .endpoints
        .iter()
        .map(|e| {
            (
                endpoint_key(e),
                serde_json::to_string(e).unwrap_or_default(),
            )
        })
        .collect();

    let mut diff = SnapshotDiff::default();
    for (k, v) in &new_map {
        match old_map.get(k) {
            None => diff.added.push(k.clone()),
            Some(old_v) if old_v != v => diff.modified.push(k.clone()),
            _ => {}
        }
    }
    for k in old_map.keys() {
        if !new_map.contains_key(k) {
            diff.removed.push(k.clone());
        }
    }
    diff.added.sort();
    diff.removed.sort();
    diff.modified.sort();
    diff
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{Endpoint, EndpointStatus, HttpMethod, Project, RequestSpec};

    fn snapshot(paths: &[&str]) -> BackupFile {
        let project = Project {
            id: uuid::Uuid::new_v4(),
            name: "P".into(),
            description: String::new(),
            variables: Default::default(),
            team_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let endpoints = paths
            .iter()
            .map(|p| Endpoint {
                id: uuid::Uuid::new_v4(),
                project_id: project.id,
                folder_id: None,
                name: p.to_string(),
                method: HttpMethod::GET,
                path: p.to_string(),
                description: String::new(),
                status: EndpointStatus::Released,
                sort_order: 0,
                request: RequestSpec::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .collect();
        BackupFile {
            format: miyo_backup::FORMAT.to_string(),
            schema_version: miyo_backup::SCHEMA_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            project,
            folders: vec![],
            endpoints,
            environments: vec![],
            mock_rules: vec![],
            response_examples: vec![],
            request_examples: vec![],
            proto_files: vec![],
            settings: Default::default(),
            global_variables: vec![],
            global_params: vec![],
            team: None,
            scenarios: vec![],
        }
    }

    #[test]
    fn diff_added_removed_modified() {
        let old = snapshot(&["/a", "/b"]);
        let mut new = snapshot(&["/b", "/c"]);
        // /b 修改：改 name。
        new.endpoints[0].name = "b-v2".into();

        let d = diff_snapshots(Some(&old), &new);
        assert_eq!(d.added, ["GET /c"]);
        assert_eq!(d.removed, ["GET /a"]);
        assert_eq!(d.modified, ["GET /b"]);

        // 无旧快照：全部为新增。
        let d = diff_snapshots(None, &new);
        assert_eq!(d.added.len(), 2);
    }
}
