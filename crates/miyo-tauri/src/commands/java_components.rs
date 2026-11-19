//! Java Attach 组件自动下载：agent / launcher jar 的状态查询与远程获取。
//!
//! 背景：安装包只内置核心能力；Attach 组件（两个 jar，合计 < 0.6 MiB）按需
//! 从官方 Release 下载到 `{data_dir}/java/`（候选目录首位，见 lib.rs）。
//!
//! 完整性：jar 与 `.sha256` sidecar 同源下载、哈希比对通过才落盘
//! （tmp + rename 原子写）；任一获取失败整体失败——不落未校验文件。
//!
//! 触发时机：插件激活时前端自动调 `java_component_status`，缺失即自动
//! `java_component_download`；ProcessPanel 提供手动重试按钮。

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use miyo_java_attach::{AGENT_JAR_NAME, LAUNCHER_JAR_NAME};
use miyo_storage::repository as repo;

use crate::commands::java_gate::ensure_java_plugin;
use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// 官方 Release 组件基址（`releases/latest/download` 稳定链接；jar 内协议
/// 握手有版本校验兜底）。
const COMPONENT_BASE_URL: &str = "https://github.com/weihubeats/Miyo/releases/latest/download";

/// 组件落地目录（候选目录首位）。
pub(crate) fn component_base_dir() -> PathBuf {
    miyo_storage::db::data_dir().join("java")
}

fn component_names() -> [&'static str; 2] {
    [AGENT_JAR_NAME, LAUNCHER_JAR_NAME]
}

fn component_url(name: &str) -> String {
    format!("{COMPONENT_BASE_URL}/{name}")
}

/// 组件状态视图。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JavaComponentItemView {
    /// 组件文件名（miyo-java-agent.jar / miyo-attach-launcher.jar）。
    pub name: String,
    /// 是否已在任一候选目录就绪。
    pub present: bool,
    /// 就绪路径（缺失为 None）。
    pub path: Option<String>,
    /// 远程下载地址（缺失时展示 / 手动下载用）。
    pub download_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JavaComponentStatusView {
    /// 全部组件就绪（attach 可用）。
    pub ready: bool,
    /// 组件落地目录（下载目标）。
    pub base_dir: String,
    pub items: Vec<JavaComponentItemView>,
}

/// 查找组件就绪位置（候选序：落地目录优先，其后为随包 / dev 目录）。
fn find_component(candidates: &[PathBuf], name: &str) -> Option<PathBuf> {
    let base = component_base_dir();
    std::iter::once(&base)
        .chain(candidates.iter())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

fn status_view(candidates: &[PathBuf]) -> JavaComponentStatusView {
    let base = component_base_dir();
    let items: Vec<JavaComponentItemView> = component_names()
        .into_iter()
        .map(|name| {
            let found = find_component(candidates, name);
            JavaComponentItemView {
                name: name.to_string(),
                present: found.is_some(),
                path: found.map(|p| p.display().to_string()),
                download_url: component_url(name),
            }
        })
        .collect();
    let ready = items.iter().all(|i| i.present);
    JavaComponentStatusView {
        ready,
        base_dir: base.display().to_string(),
        items,
    }
}

/// 组件状态查询（门禁：插件已安装；未启用也允许查——启用流程需要它）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_component_status(
    state: State<'_, AppState>,
) -> CommandResult<JavaComponentStatusView> {
    Ok(status_view(&state.java_attach.jar_dirs()))
}

/// 下载缺失组件（幂等：已就绪的跳过；门禁：插件已启用）。
#[tauri::command(rename_all = "camelCase")]
pub async fn java_component_download(
    state: State<'_, AppState>,
) -> CommandResult<JavaComponentStatusView> {
    ensure_java_plugin(&state.db).await?;
    let candidates = state.java_attach.jar_dirs();
    let base = component_base_dir();
    std::fs::create_dir_all(&base)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;

    for name in component_names() {
        if find_component(&candidates, name).is_some() {
            continue;
        }
        fetch_and_store_component(&base, name, &component_url(name)).await?;
        if let Err(e) = repo::insert_plugin_log(
            &state.db,
            crate::commands::java_gate::JAVA_PLUGIN_ID,
            "audit",
            &format!("component download {name}"),
            None,
        )
        .await
        {
            tracing::warn!("组件下载审计失败：{e}");
        }
    }

    let status = status_view(&candidates);
    audit_summary(&state.db, &status).await;
    Ok(status)
}

/// 获取单个组件并原子落盘（jar + sha256 sidecar 同源下载，哈希比对通过才写）。
pub(crate) async fn fetch_and_store_component(
    base_dir: &Path,
    name: &str,
    base_url: &str,
) -> CommandResult<()> {
    let jar = crate::commands::plugins::download_package(&format!("{base_url}/{name}")).await?;
    let sha_raw =
        crate::commands::plugins::download_package(&format!("{base_url}/{name}.sha256")).await?;
    let expected = parse_sha256_file(&sha_raw)
        .ok_or_else(|| CommandError::validation(format!("{name} 的 sha256 sidecar 格式非法")))?;
    let mut hasher = Sha256::new();
    hasher.update(&jar);
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(CommandError::validation(format!(
            "{name} 哈希校验失败（期望 {expected}，实际 {actual}）——下载可能被篡改，已拒绝落盘"
        )));
    }
    // 原子写：tmp → rename（防半截文件被 attach 使用）
    let tmp = base_dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, &jar)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    let dest = base_dir.join(name);
    std::fs::rename(&tmp, &dest)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    Ok(())
}

/// 下载结果摘要审计（一行：各组件就绪态）。
async fn audit_summary(db: &sqlx::SqlitePool, status: &JavaComponentStatusView) {
    let summary = status
        .items
        .iter()
        .map(|i| format!("{}={}", i.name, if i.present { "ok" } else { "missing" }))
        .collect::<Vec<_>>()
        .join(", ");
    if let Err(e) = repo::insert_plugin_log(
        db,
        crate::commands::java_gate::JAVA_PLUGIN_ID,
        "audit",
        &format!("components ready={} ({summary})", status.ready),
        None,
    )
    .await
    {
        tracing::warn!("组件状态审计失败：{e}");
    }
}

/// 解析 sha256sum 格式 sidecar（`<hex>  <filename>`，取首个 64-hex token）。
fn parse_sha256_file(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let token = text.split_whitespace().next()?.trim();
    if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(token.to_ascii_lowercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_file_parsing() {
        let hex = "ab".repeat(32);
        // sha256sum 标准格式：<hex>  <name>
        let raw = format!("{hex}  miyo-java-agent.jar\n");
        assert_eq!(
            parse_sha256_file(raw.as_bytes()).as_deref(),
            Some(hex.as_str())
        );
        // 裸 hex
        assert_eq!(
            parse_sha256_file(hex.as_bytes()).as_deref(),
            Some(hex.as_str())
        );
        // 非法：长度不足 / 非十六进制 / 空
        assert!(parse_sha256_file(b"abcd").is_none());
        assert!(parse_sha256_file(&b"zz".repeat(32)).is_none());
        assert!(parse_sha256_file(b"").is_none());
    }

    /// 真实下载链路：mock HTTP（jar + sha256 sidecar）→ fetch_and_store_component
    /// 哈希比对 → 原子落盘；篡改 sidecar → 拒绝落盘。
    #[tokio::test]
    async fn fetch_and_store_verifies_hash_before_write() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let jar: &'static [u8] = Box::leak(b"FAKE-JAR-CONTENT".to_vec().into_boxed_slice());
        let digest: &'static str = Box::leak(hex::encode(Sha256::digest(jar)).into_boxed_str());

        // 通用 mock：按路径返回 jar / sha256 / 篡改 sha256
        async fn spawn_mock(jar: &'static [u8], sha: &'static str) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    if let Ok((mut sock, _)) = listener.accept().await {
                        let mut buf = vec![0u8; 4096];
                        let _ = sock.read(&mut buf).await;
                        let path = String::from_utf8_lossy(&buf).to_string();
                        let (ctype, body): (&'static str, &'static [u8]) =
                            if path.contains(".sha256") {
                                (
                                    "text/plain",
                                    Box::leak(sha.as_bytes().to_vec().into_boxed_slice()),
                                )
                            } else {
                                ("application/java-archive", jar)
                            };
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {ctype}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(body).await;
                        let _ = sock.shutdown().await;
                    }
                }
            });
            format!("http://{addr}")
        }

        let tmp = tempfile::tempdir().unwrap();
        // 正常：jar + 匹配的 sha256 → 落盘
        let base_ok = spawn_mock(jar, &digest).await;
        fetch_and_store_component(tmp.path(), AGENT_JAR_NAME, &base_ok)
            .await
            .unwrap();
        let stored = tmp.path().join(AGENT_JAR_NAME);
        assert_eq!(std::fs::read(&stored).unwrap(), jar);
        // 无 .tmp 残留（原子 rename）
        assert!(!tmp.path().join(format!(".{AGENT_JAR_NAME}.tmp")).exists());

        // 篡改：sidecar 哈希与 jar 不符 → 拒绝 + 不落盘
        let bad_digest: &'static str =
            Box::leak(hex::encode(Sha256::digest(b"tampered")).into_boxed_str());
        let base_bad = spawn_mock(jar, &bad_digest).await;
        let err = fetch_and_store_component(tmp.path(), LAUNCHER_JAR_NAME, &base_bad)
            .await
            .unwrap_err();
        assert!(err.message.contains("哈希校验失败"), "{err:?}");
        assert!(!tmp.path().join(LAUNCHER_JAR_NAME).exists());
    }
}
