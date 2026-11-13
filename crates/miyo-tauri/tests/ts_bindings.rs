//! TS 绑定导出测试：`cargo test --features ts`（miyo-tauri 工作区内执行）时把
//! CommandError 与关键 view DTO 导出到 `frontend/src/types/generated/`。
//!
//! miyo-core 的模型由其自身的 `ts_bindings.rs` 导出；本文件只覆盖 IPC 层
//! 手写 view 类型。CI 的 `bindings` job 重新导出后 `git diff --exit-code` 防漂移。
#![cfg(feature = "ts")]

use std::path::PathBuf;

use ts_rs::TS;

fn export_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../frontend/src/types/generated")
}

#[test]
fn export_bindings() {
    let cfg = ts_rs::Config::default()
        .with_out_dir(export_dir())
        .with_large_int("number");

    miyo_tauri::error::CommandError::export_all(&cfg).unwrap();
    miyo_tauri::commands::request::ExecuteResponse::export_all(&cfg).unwrap();
    miyo_tauri::commands::scenarios::ScenarioLibraryItem::export_all(&cfg).unwrap();
}
