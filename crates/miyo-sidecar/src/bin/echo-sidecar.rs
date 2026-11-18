//! echo-sidecar：Sidecar 协议参考实现（真实可执行进程，非 mock）。
//!
//! 双重用途：
//! 1. **插件开发示例**：第三方插件 sidecar 只需实现同样的协议（stdin/stdout
//!    4 字节 LE 长度前缀 + JSON 帧）——见 docs/plugin/SIDECAR.md；
//! 2. **Supervisor 集成测试目标**：`cargo test -p miyo-sidecar` 以
//!    `CARGO_BIN_EXE_echo-sidecar` 直接拉起本进程做全链路验证
//!    （握手 / 请求 / 事件 / 崩溃恢复 / 心跳判死 / 进程组清理）。
//!
//! 支持的方法：
//! - `echo`        → 回显 params（连通性验证）
//! - `crash`       → 立即 exit(1)（崩溃恢复测试）
//! - `sleep`       → 按参数 ms 延迟后回显（超时测试）
//! - `mute`        → 进入静默模式：不再响应 ping（心跳判死测试）
//!
//! 事件：`hello_ack` 后主动推送一条 `boot` 事件与一条 log（链路验证）。

use std::io::{Read, Write};

const PROTOCOL_VERSION: u32 = 1;
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut muted = false;
    // 握手前先读 hello（协议要求首帧必须 hello）
    loop {
        let frame = match read_frame(&stdin) {
            Ok(f) => f,
            Err(_) => return, // EOF / 损坏 → 退出（宿主 wait 任务检测）
        };
        let kind = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "hello" => {
                let token = frame
                    .pointer("/params/token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let protocol = frame
                    .pointer("/params/protocol")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let ack = serde_json::json!({
                    "v": PROTOCOL_VERSION,
                    "type": "hello_ack",
                    "params": {
                        "token": token,
                        "protocol": protocol,
                        "sidecar_version": env!("CARGO_PKG_VERSION"),
                    },
                });
                write_frame(&mut out, &ack);
                // 主动事件 + 日志（宿主端到端链路验证）
                write_frame(
                    &mut out,
                    &serde_json::json!({"v":1,"type":"evt","event":"boot","payload":{"pid":std::process::id()}}),
                );
                write_frame(
                    &mut out,
                    &serde_json::json!({"v":1,"type":"log","level":"info","message":"echo-sidecar ready"}),
                );
            }
            "ping" => {
                if !muted {
                    let id = frame.get("id").cloned().unwrap_or(serde_json::Value::Null);
                    write_frame(&mut out, &serde_json::json!({"v":1,"type":"pong","id":id}));
                }
            }
            "req" => {
                let id = frame.get("id").cloned().unwrap_or(serde_json::Value::Null);
                let method = frame
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let params = frame
                    .get("params")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                match method.as_str() {
                    "echo" => {
                        write_frame(
                            &mut out,
                            &serde_json::json!({"v":1,"type":"res","id":id,"ok":true,"result":{"echo":params}}),
                        );
                    }
                    "crash" => {
                        // 直接退出（模拟崩溃；不回 res 帧）
                        std::process::exit(1);
                    }
                    "sleep" => {
                        let ms = params.get("ms").and_then(|v| v.as_u64()).unwrap_or(100);
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                        write_frame(
                            &mut out,
                            &serde_json::json!({"v":1,"type":"res","id":id,"ok":true,"result":{"slept":ms}}),
                        );
                    }
                    "mute" => {
                        muted = true;
                        write_frame(
                            &mut out,
                            &serde_json::json!({"v":1,"type":"res","id":id,"ok":true,"result":"muted"}),
                        );
                    }
                    "fail" => {
                        write_frame(
                            &mut out,
                            &serde_json::json!({"v":1,"type":"res","id":id,"ok":false,
                                "error":{"code":"BUSINESS","message":"业务失败示例"}}),
                        );
                    }
                    _ => {
                        write_frame(
                            &mut out,
                            &serde_json::json!({"v":1,"type":"res","id":id,"ok":false,
                                "error":{"code":"NOT_FOUND","message":format!("未知方法 {method}")}}),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// 读一帧（4 字节 LE 长度前缀 + JSON）。
fn read_frame(stdin: &std::io::Stdin) -> Result<serde_json::Value, ()> {
    let mut lock = stdin.lock();
    let mut header = [0u8; 4];
    lock.read_exact(&mut header).map_err(|_| ())?;
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        return Err(());
    }
    let mut body = vec![0u8; len];
    lock.read_exact(&mut body).map_err(|_| ())?;
    serde_json::from_slice(&body).map_err(|_| ())
}

/// 写一帧。
fn write_frame(out: &mut std::io::Stdout, frame: &serde_json::Value) {
    let body = serde_json::to_vec(frame).expect("frame serialize");
    let mut lock = out.lock();
    let _ = lock.write_all(&(body.len() as u32).to_le_bytes());
    let _ = lock.write_all(&body);
    let _ = lock.flush();
}
