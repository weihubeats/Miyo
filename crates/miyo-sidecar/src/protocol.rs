//! Sidecar stdio 协议：4 字节小端长度前缀 + UTF-8 JSON 帧。
//!
//! 帧类型（v = 协议版本，破坏性变更递增）：
//! - 宿主 → sidecar：`hello`（握手，含一次性会话 token）、`ping`、`req`
//! - sidecar → 宿主：`hello_ack`（token 回显）、`pong`、`res`、`evt`、`log`
//!
//! 帧上限 8 MiB；超限即断链（与解析失败同等处理）。

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// sidecar 协议版本（manifest.sidecar.protocol 必须一致）。
pub const SIDECAR_PROTOCOL_VERSION: u32 = 1;
/// 单帧字节上限。
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// 协议错误。
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("sidecar 帧超限（{0} 字节 > 上限 {MAX}）", MAX = MAX_FRAME_BYTES)]
    TooLarge(usize),
    #[error("sidecar 流意外关闭")]
    Eof,
    #[error("sidecar 帧 IO 失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("sidecar 帧解析失败：{0}")]
    Json(#[from] serde_json::Error),
}

/// hello 帧（宿主 → sidecar；token 单次会话凭证）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloFrame {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub plugin_id: String,
    pub token: String,
    /// 宿主支持的 sidecar 协议版本。
    pub protocol: u32,
}

/// hello_ack 帧（sidecar → 宿主；token 回显必须一致）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAckFrame {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: String,
    pub token: String,
    /// sidecar 接受的协议版本。
    pub protocol: u32,
    pub sidecar_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 通用帧（ping/pong/req/res/evt/log 共用形状，`kind` 区分）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarFrame {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<SidecarErrorBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarErrorBody {
    pub code: String,
    pub message: String,
}

/// 从流读取一帧（4 字节 LE 长度前缀 + JSON）。
pub async fn read_frame<R: AsyncReadExt + Unpin>(
    stream: &mut R,
) -> Result<SidecarFrame, FrameError> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            FrameError::Eof
        } else {
            FrameError::Io(e)
        }
    })?;
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

/// 写一帧到流（长度前缀 + JSON）。
pub async fn write_frame<W: AsyncWriteExt + Unpin>(
    stream: &mut W,
    frame: &SidecarFrame,
) -> Result<(), FrameError> {
    let body = serde_json::to_vec(frame)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body.len()));
    }
    stream.write_all(&(body.len() as u32).to_le_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

/// 构造帧（宿主侧便捷函数）。
pub fn host_frame(kind: &str) -> SidecarFrame {
    SidecarFrame {
        v: SIDECAR_PROTOCOL_VERSION,
        kind: kind.to_string(),
        id: None,
        method: None,
        params: None,
        ok: None,
        result: None,
        error: None,
        event: None,
        payload: None,
        level: None,
        message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 帧编解码 roundtrip（内存 duplex）。
    #[tokio::test]
    async fn frame_roundtrip() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let frame = host_frame("req");
        let writer = tokio::spawn(async move {
            write_frame(&mut client, &frame).await.unwrap();
        });
        let read = read_frame(&mut server).await.unwrap();
        writer.await.unwrap();
        assert_eq!(read.kind, "req");
        assert_eq!(read.v, SIDECAR_PROTOCOL_VERSION);
    }

    /// 超限帧拒绝（伪造长度头）。
    #[tokio::test]
    async fn oversized_frame_rejected() {
        let (mut client, mut server) = tokio::io::duplex(64);
        client
            .write_all(&((MAX_FRAME_BYTES + 1) as u32).to_le_bytes())
            .await
            .unwrap();
        let err = read_frame(&mut server).await.unwrap_err();
        assert!(matches!(err, FrameError::TooLarge(_)));
    }

    /// EOF → 结构化错误（子进程退出检测路径）。
    #[tokio::test]
    async fn eof_detected() {
        let (client, mut server) = tokio::io::duplex(64);
        drop(client);
        let err = read_frame(&mut server).await.unwrap_err();
        assert!(matches!(err, FrameError::Eof));
    }
}
