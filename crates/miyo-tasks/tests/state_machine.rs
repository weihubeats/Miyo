//! TaskSupervisor 状态机 / 幂等 / RAII / 资源键守卫 / 历史测试。

use std::sync::Arc;

use miyo_tasks::{CancelOutcome, TaskKind, TaskState, TaskSupervisor};

fn sup() -> Arc<TaskSupervisor> {
    TaskSupervisor::new()
}

#[test]
fn state_machine_running_to_completed() {
    let s = sup();
    let handle = s.start("t1", TaskKind::LoadTest, None).unwrap();
    assert_eq!(s.status("t1").unwrap().state, TaskState::Running);
    handle.mark_completed();
    assert!(s.status("t1").is_none());
    assert_eq!(s.history(TaskKind::LoadTest).len(), 1);
    assert_eq!(s.history(TaskKind::LoadTest)[0].state, TaskState::Completed);
}

#[test]
fn cancel_idempotent_and_not_found() {
    let s = sup();
    let _handle = s.start("t2", TaskKind::ScenarioRun, None).unwrap();
    assert_eq!(s.cancel("t2"), CancelOutcome::Cancelled);
    assert_eq!(s.cancel("t2"), CancelOutcome::AlreadyFinished);
    assert_eq!(s.cancel("nope"), CancelOutcome::NotFound);
    assert_eq!(
        s.history(TaskKind::ScenarioRun)[0].state,
        TaskState::Cancelled
    );
}

#[test]
fn raii_drop_marks_cancelled() {
    let s = sup();
    {
        let _handle = s.start("t3", TaskKind::AiChat, None).unwrap();
        assert!(s.status("t3").is_some());
    }
    assert!(s.status("t3").is_none());
    assert_eq!(s.history(TaskKind::AiChat)[0].state, TaskState::Cancelled);
}

#[test]
fn resource_key_guard_rejects_reentry() {
    let s = sup();
    let h1 = s
        .start("m1", TaskKind::ScenarioMatrix, Some("scenario:42".into()))
        .unwrap();
    assert!(s
        .start("m2", TaskKind::ScenarioMatrix, Some("scenario:42".into()))
        .is_err());
    h1.mark_completed();
    assert!(s
        .start("m3", TaskKind::ScenarioMatrix, Some("scenario:42".into()))
        .is_ok());
}

#[test]
fn duplicate_id_replaces_and_cancels_old() {
    let s = sup();
    let h1 = s.start("dup", TaskKind::HttpRequest, None).unwrap();
    let h2 = s.start("dup", TaskKind::HttpRequest, None).unwrap();
    assert!(h1.cancellation_token().is_cancelled());
    assert!(!h2.cancellation_token().is_cancelled());
    h2.mark_completed();
}

#[test]
fn cancellation_token_shared_with_task_body() {
    let s = sup();
    let handle = s.start("tok", TaskKind::LoadTest, None).unwrap();
    let token = handle.cancellation_token();
    assert!(!token.is_cancelled());
    s.cancel("tok");
    assert!(token.is_cancelled());
}

#[test]
fn history_ring_buffer_capped() {
    let s = sup();
    for i in 0..(miyo_tasks::HISTORY_CAP + 20) {
        let h = s
            .start(format!("h{i}"), TaskKind::TestEndpoint, None)
            .unwrap();
        h.mark_completed();
    }
    assert_eq!(
        s.history(TaskKind::TestEndpoint).len(),
        miyo_tasks::HISTORY_CAP
    );
}

#[test]
fn failed_state_records_error() {
    let s = sup();
    let h = s.start("f1", TaskKind::GrpcStream, None).unwrap();
    h.mark_failed("连接中断");
    let rec = &s.history(TaskKind::GrpcStream)[0];
    assert_eq!(rec.state, TaskState::Failed);
    assert_eq!(rec.last_error.as_deref(), Some("连接中断"));
}

#[test]
fn shutdown_all_cancels_everything() {
    let s = sup();
    let a = s.start("s1", TaskKind::LoadTest, None).unwrap();
    let b = s.start("s2", TaskKind::WsSession, None).unwrap();
    let ta = a.cancellation_token();
    let tb = b.cancellation_token();
    s.shutdown_all();
    assert!(ta.is_cancelled() && tb.is_cancelled());
}
