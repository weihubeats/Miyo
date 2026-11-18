//! 插件会话凭证（std 族能力的后端强制凭据）。
//!
//! 模型：插件激活（或管理界面打开详情）时经 `plugin_session_create` 换取
//! 一次性 token；后续 `plugin_data_*` / `plugin_record_log` 等 std 族命令必须
//! 携带 token，后端校验 `token ↔ plugin_id` 绑定与有效期。
//!
//! 生命周期：
//! - 有效期固定 [`SESSION_TTL`]，过期即失效（validate 时惰性清理）；
//! - 收权（plugin_set_permissions）/ 停用 / 卸载 → 吊销该插件全部会话
//!   （撤销即时生效）；
//! - 每插件并存上限 [`MAX_SESSIONS_PER_PLUGIN`]（UI 详情页与 SDK 并发使用
//!   互不影响），超限吊销最旧会话。
//!
//! 会话表纯内存（不落库）：Miyo 重启后由前端重新 create，无恢复语义。

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 会话有效期。
pub const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
/// 每插件并存会话上限。
pub const MAX_SESSIONS_PER_PLUGIN: usize = 16;

/// 校验失败原因（命令层映射为 FORBIDDEN）。
#[derive(Debug)]
pub enum SessionError {
    /// token 不存在 / 已过期 / 已吊销。
    Unknown,
    /// token 与请求的 plugin_id 不匹配（跨插件盗用）。
    Mismatch,
}

struct SessionEntry {
    plugin_id: String,
    /// 签发时的权限快照（收权后吊销重发，因此这里无需实时更新）。
    #[allow(dead_code)]
    granted: Vec<String>,
    expires_at: Instant,
}

/// 会话注册表（AppState 持有；全部走 &self + 内部锁）。
pub struct PluginSessions {
    inner: std::sync::Mutex<HashMap<String, SessionEntry>>,
}

impl Default for PluginSessions {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginSessions {
    pub fn new() -> Self {
        PluginSessions {
            inner: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// 签发新会话（返回 256-bit hex token）。同插件超限吊销最旧会话。
    pub fn create(&self, plugin_id: &str, granted: Vec<String>) -> String {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut inner = self.inner.lock().unwrap();
        // 同插件会话计数（惰性清过期后统计）
        let mut live: Vec<(String, Instant)> = inner
            .iter()
            .filter(|(_, e)| e.plugin_id == plugin_id)
            .map(|(t, e)| (t.clone(), e.expires_at))
            .collect();
        while live.len() >= MAX_SESSIONS_PER_PLUGIN {
            let oldest = live
                .iter()
                .min_by_key(|(_, exp)| *exp)
                .map(|(t, _)| t.clone())
                .expect("live 非空");
            inner.remove(&oldest);
            live.retain(|(t, _)| *t != oldest);
        }
        inner.insert(
            token.clone(),
            SessionEntry {
                plugin_id: plugin_id.to_string(),
                granted,
                expires_at: Instant::now() + SESSION_TTL,
            },
        );
        token
    }

    /// 校验 token 与 plugin_id 绑定（过期 / 未知 / 不匹配分别报错）。
    pub fn validate(&self, token: &str, plugin_id: &str) -> Result<(), SessionError> {
        let mut inner = self.inner.lock().unwrap();
        let Some(entry) = inner.get(token) else {
            return Err(SessionError::Unknown);
        };
        if entry.expires_at <= Instant::now() {
            inner.remove(token);
            return Err(SessionError::Unknown);
        }
        if entry.plugin_id != plugin_id {
            return Err(SessionError::Mismatch);
        }
        Ok(())
    }

    /// 吊销某插件的全部会话（收权 / 停用 / 卸载时调用）。
    pub fn revoke_plugin(&self, plugin_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.retain(|_, e| e.plugin_id != plugin_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_validate_roundtrip() {
        let s = PluginSessions::new();
        let t = s.create("com.a.x", vec!["std.storage".into()]);
        s.validate(&t, "com.a.x").unwrap();
    }

    #[test]
    fn cross_plugin_use_rejected() {
        let s = PluginSessions::new();
        let t = s.create("com.a.x", vec![]);
        assert!(matches!(
            s.validate(&t, "com.b.y"),
            Err(SessionError::Mismatch)
        ));
    }

    #[test]
    fn unknown_token_rejected() {
        let s = PluginSessions::new();
        assert!(matches!(
            s.validate("nope", "com.a.x"),
            Err(SessionError::Unknown)
        ));
    }

    #[test]
    fn revoke_plugin_invalidates_all_sessions() {
        let s = PluginSessions::new();
        let t1 = s.create("com.a.x", vec![]);
        let t2 = s.create("com.a.x", vec![]);
        s.revoke_plugin("com.a.x");
        assert!(matches!(
            s.validate(&t1, "com.a.x"),
            Err(SessionError::Unknown)
        ));
        assert!(matches!(
            s.validate(&t2, "com.a.x"),
            Err(SessionError::Unknown)
        ));
    }

    #[test]
    fn per_plugin_cap_evicts_oldest() {
        let s = PluginSessions::new();
        let mut tokens = Vec::new();
        for _ in 0..MAX_SESSIONS_PER_PLUGIN {
            tokens.push(s.create("com.a.x", vec![]));
        }
        // 再签发一个：总数超限 → 最旧（第一个）被吊销
        let extra = s.create("com.a.x", vec![]);
        assert!(matches!(
            s.validate(&tokens[0], "com.a.x"),
            Err(SessionError::Unknown)
        ));
        s.validate(&tokens[1], "com.a.x").unwrap();
        s.validate(&extra, "com.a.x").unwrap();
    }
}
