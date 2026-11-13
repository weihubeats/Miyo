//! TS 绑定导出测试：`cargo test -p miyo-core --features ts` 时把公共模型
//! 导出到 `frontend/src/types/generated/`，供 `miyoApi.d.ts` re-export。
//!
//! 根类型清单 + `export_all`（递归拉齐全部依赖类型）覆盖 model.rs 的全部
//! 可序列化公共类型；CI 的 `bindings` job 重新导出后 `git diff --exit-code`
//! 防手写镜像漂移（Rust 模型改动若未同步前端类型，此处红灯）。
//!
//! - `large_int = "number"`：IPC JSON 里 i64/u64 本就是 number，避免 bigint 误报；
//! - 目录锚定 `CARGO_MANIFEST_DIR`，与执行 cargo 的 CWD 无关。
#![cfg(feature = "ts")]

use std::path::PathBuf;

use miyo_core::model::{
    Endpoint, Environment, EnvironmentVariable, Folder, GlobalParam, GraphQLResponse,
    MockRequestLog, MockRule, Project, ProtoFile, RequestExample, RequestHistory, RequestSpec,
    ResponseExample, Scenario, ScenarioRun, ScenarioStep, SeqCounter, SyncRemote, Team, TestCase,
    TestCaseRun, TestCaseRunSummary, TestRun, WsMessageRecord,
};
use ts_rs::TS;

fn export_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../frontend/src/types/generated")
}

#[test]
fn export_bindings() {
    let cfg = ts_rs::Config::default()
        .with_out_dir(export_dir())
        .with_large_int("number");

    // 根类型：export_all 会把字段/嵌套引用到的类型一并导出
    //（KeyValue/AuthSpec/BodySpec/ScenarioInlineRequest/StepRetry/DataSet/
    //  FieldDoc/GraphQLSpec/GrpcSpec 等 53 个类型均由此拉齐）。
    RequestSpec::export_all(&cfg).unwrap();
    Endpoint::export_all(&cfg).unwrap();
    Project::export_all(&cfg).unwrap();
    Folder::export_all(&cfg).unwrap();
    Team::export_all(&cfg).unwrap();
    TestCase::export_all(&cfg).unwrap();
    TestCaseRun::export_all(&cfg).unwrap();
    TestCaseRunSummary::export_all(&cfg).unwrap();
    TestRun::export_all(&cfg).unwrap();
    Scenario::export_all(&cfg).unwrap();
    ScenarioStep::export_all(&cfg).unwrap();
    ScenarioRun::export_all(&cfg).unwrap();
    Environment::export_all(&cfg).unwrap();
    EnvironmentVariable::export_all(&cfg).unwrap();
    GlobalParam::export_all(&cfg).unwrap();
    MockRule::export_all(&cfg).unwrap();
    MockRequestLog::export_all(&cfg).unwrap();
    SyncRemote::export_all(&cfg).unwrap();
    RequestHistory::export_all(&cfg).unwrap();
    RequestExample::export_all(&cfg).unwrap();
    ResponseExample::export_all(&cfg).unwrap();
    GraphQLResponse::export_all(&cfg).unwrap();
    WsMessageRecord::export_all(&cfg).unwrap();
    SeqCounter::export_all(&cfg).unwrap();
    ProtoFile::export_all(&cfg).unwrap();
}
