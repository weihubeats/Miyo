//! TS 绑定导出测试：`cargo test -p miyo-test --features ts` 导出压测配置 /
//! 结果 / 进度类型到 `frontend/src/types/generated/`。CI bindings job 防漂移。
#![cfg(feature = "ts")]

use std::path::PathBuf;

use miyo_test::load::{LoadConfig, LoadProgress, LoadResult, LoadSchedule};
use ts_rs::TS;

fn export_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../frontend/src/types/generated")
}

#[test]
fn export_bindings() {
    let cfg = ts_rs::Config::default()
        .with_out_dir(export_dir())
        .with_large_int("number");

    LoadConfig::export_all(&cfg).unwrap();
    LoadSchedule::export_all(&cfg).unwrap();
    LoadResult::export_all(&cfg).unwrap();
    LoadProgress::export_all(&cfg).unwrap();
}
