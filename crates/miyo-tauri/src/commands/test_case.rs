//! 测试用例 Command：列表（按接口）/ 保存 / 更新（名称·分组·请求内容·运行状态）/ 删除。

use tauri::State;
use uuid::Uuid;

use miyo_core::model::{HttpMethod, KeyValue, TestCase, TestCaseStatus};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 允许的用例分组。
const CATEGORIES: [&str; 5] = ["正向", "负向", "边界值", "安全性", "其他"];

/// 列出接口的全部测试用例。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_test_cases(
    state: State<'_, AppState>,
    request_id: Uuid,
) -> CommandResult<Vec<TestCase>> {
    repo::list_test_cases(&state.db, request_id)
        .await
        .map_err(Into::into)
}

/// 保存测试用例（新建）。名称必填、分组须在允许集合内。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_test_case(
    state: State<'_, AppState>,
    test_case: TestCase,
) -> CommandResult<TestCase> {
    if test_case.name.trim().is_empty() {
        return Err(CommandError::validation("用例名称不能为空"));
    }
    if !CATEGORIES.contains(&test_case.category.as_str()) {
        return Err(CommandError::validation(format!(
            "无效的用例分组：{}",
            test_case.category
        )));
    }
    repo::create_test_case(&state.db, &test_case).await?;
    Ok(test_case)
}

/// 更新用例名称 / 分组 / 文件夹（编辑入口）。文件夹为自由分组名，空 = 未分组。
#[tauri::command(rename_all = "camelCase")]
pub async fn update_test_case_meta(
    state: State<'_, AppState>,
    case_id: Uuid,
    name: String,
    category: String,
    folder: String,
) -> CommandResult<()> {
    if name.trim().is_empty() {
        return Err(CommandError::validation("用例名称不能为空"));
    }
    if !CATEGORIES.contains(&category.as_str()) {
        return Err(CommandError::validation(format!(
            "无效的用例分组：{category}"
        )));
    }
    let folder = folder.trim().to_string();
    if folder.chars().count() > 60 {
        return Err(CommandError::validation("文件夹名称最长 60 字符"));
    }
    repo::update_test_case_meta(&state.db, case_id, &name, &category, &folder)
        .await
        .map_err(Into::into)
}

/// 批量移动用例到文件夹（空 = 移出分组；行菜单「移动到分组」）。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_test_case_folder(
    state: State<'_, AppState>,
    case_ids: Vec<Uuid>,
    folder: String,
) -> CommandResult<u64> {
    if case_ids.is_empty() {
        return Err(CommandError::validation("未选择用例"));
    }
    let folder = folder.trim().to_string();
    if folder.chars().count() > 60 {
        return Err(CommandError::validation("文件夹名称最长 60 字符"));
    }
    repo::set_test_cases_folder(&state.db, &case_ids, &folder)
        .await
        .map_err(Into::into)
}

/// 重命名接口下的一个用例文件夹（整组改名；目标名已存在时两组合并）。
#[tauri::command(rename_all = "camelCase")]
pub async fn rename_test_case_folder(
    state: State<'_, AppState>,
    request_id: Uuid,
    old_folder: String,
    new_folder: String,
) -> CommandResult<u64> {
    let old_folder = old_folder.trim().to_string();
    let new_folder = new_folder.trim().to_string();
    if old_folder.is_empty() || new_folder.is_empty() {
        return Err(CommandError::validation("文件夹名称不能为空"));
    }
    if old_folder == new_folder {
        return Ok(0);
    }
    if new_folder.chars().count() > 60 {
        return Err(CommandError::validation("文件夹名称最长 60 字符"));
    }
    repo::rename_test_case_folder(&state.db, request_id, &old_folder, &new_folder)
        .await
        .map_err(Into::into)
}

/// 更新用例运行状态（Success / Failed / Untested）。
#[tauri::command(rename_all = "camelCase")]
pub async fn update_test_case_status(
    state: State<'_, AppState>,
    case_id: Uuid,
    status: TestCaseStatus,
) -> CommandResult<()> {
    repo::update_test_case_status(&state.db, case_id, status)
        .await
        .map_err(Into::into)
}

/// 更新用例完整请求内容（方法 / 路径 / 路径变量 / Params / Headers / Body，抽屉「保存修改」）。
#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn update_test_case_content(
    state: State<'_, AppState>,
    case_id: Uuid,
    method: HttpMethod,
    url_path: String,
    path_variables: Vec<KeyValue>,
    params: Vec<KeyValue>,
    headers: Vec<KeyValue>,
    body_type: String,
    body_content: String,
) -> CommandResult<()> {
    if url_path.trim().is_empty() {
        return Err(CommandError::validation("请求路径不能为空"));
    }
    repo::update_test_case_content(
        &state.db,
        case_id,
        method,
        &url_path,
        &path_variables,
        &params,
        &headers,
        &body_type,
        &body_content,
    )
    .await
    .map_err(Into::into)
}

/// 删除测试用例。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_test_case(state: State<'_, AppState>, case_id: Uuid) -> CommandResult<()> {
    repo::delete_test_case(&state.db, case_id)
        .await
        .map_err(Into::into)
}
