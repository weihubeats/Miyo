//! 项目 JSON 备份与恢复（M10）。
//!
//! 备份 = 单个 JSON 文件，包含项目及全部子对象（含 UUID 引用关系）。
//! 恢复 = 解析备份并重新分配 UUID（新项目），保证不会与现有数据冲突。

use std::collections::HashMap;

use chrono::Utc;
use miyo_core::model::{
    Endpoint, Environment, EnvironmentVariable, Folder, GlobalParam, MockRule, Project, ProtoFile,
    RequestExample, ResponseExample, Scenario, Team,
};
use miyo_core::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 备份文件（顶层）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupFile {
    pub format: String,
    pub schema_version: u32,
    pub exported_at: String,
    pub project: Project,
    pub folders: Vec<Folder>,
    pub endpoints: Vec<Endpoint>,
    pub environments: Vec<Environment>,
    pub mock_rules: Vec<MockRule>,
    pub response_examples: Vec<ResponseExample>,
    /// 请求用例（旧版本备份无此字段，缺失时按空处理）。
    #[serde(default)]
    pub request_examples: Vec<RequestExample>,
    /// 项目级 proto 文件（gRPC 调试用；旧备份缺失按空处理）。
    #[serde(default)]
    pub proto_files: Vec<ProtoFile>,
    /// 全局设置快照（白名单键：代理/超时/自增序列；旧备份缺失按空处理）。
    #[serde(default)]
    pub settings: HashMap<String, String>,
    /// 全局变量（旧备份缺失按空处理；恢复时按 key 合并，缺失才补）。
    #[serde(default)]
    pub global_variables: Vec<EnvironmentVariable>,
    /// 全局参数（旧备份缺失按空处理；恢复时按 key 合并，缺失才补）。
    #[serde(default)]
    pub global_params: Vec<GlobalParam>,
    /// 团队元数据（v4 起；旧备份缺失时为 None，项目 team_id 兜底为默认团队）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<Team>,
    /// 集成测试场景（v5 起；旧备份缺失按空处理）。步骤为接口引用式，
    /// 恢复时 steps 内的 endpoint_id 随接口重映射。
    #[serde(default)]
    pub scenarios: Vec<Scenario>,
}

/// 备份格式标识。
pub const FORMAT: &str = "miyo-project-backup";
/// 品牌更名前的格式标识：解析时同样接受，旧备份文件可继续导入恢复。
pub const LEGACY_FORMAT: &str = "rustfox-project-backup";
/// 当前 schema 版本（v5：补齐 scenarios；v4 团队 / v1-v3 环境形状照旧升级）。
pub const SCHEMA_VERSION: u32 = 5;

impl BackupFile {
    pub fn serialize(&self) -> Result<String, AppError> {
        serde_json::to_string_pretty(self).map_err(AppError::Json)
    }

    pub fn parse(text: &str) -> Result<BackupFile, AppError> {
        let mut value: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| AppError::Validation(format!("备份文件解析失败：{e}")))?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if (1..=SCHEMA_VERSION as u64).contains(&version) && version < SCHEMA_VERSION as u64 {
            upgrade_legacy_environments(&mut value);
            value["schema_version"] = serde_json::json!(SCHEMA_VERSION);
        }
        let file: BackupFile = serde_json::from_value(value)
            .map_err(|e| AppError::Validation(format!("备份文件解析失败：{e}")))?;
        if file.format != FORMAT && file.format != LEGACY_FORMAT {
            return Err(miyo_core::validation("不是有效的 Miyo 备份文件"));
        }
        if file.schema_version > SCHEMA_VERSION {
            return Err(miyo_core::validation(format!(
                "备份文件版本 {} 过新，当前最高支持 {}",
                file.schema_version, SCHEMA_VERSION
            )));
        }
        Ok(file)
    }
}

/// 旧版备份升级到当前环境形状：
///
/// - v1：环境变量为 `{key:value}` map —— 全部键转结构化数组，
///   `base_url` 键提升为环境 Base URL；
/// - v2：环境为全局维度多模块 `modules` —— 取默认 / 首个模块的
///   base_url 提升为环境 Base URL；
/// - `project_id` 占位为文件内项目 id（恢复时整体重映射到新项目）。
fn upgrade_legacy_environments(value: &mut serde_json::Value) {
    let project_id = value
        .get("project")
        .and_then(|p| p.get("id"))
        .and_then(|i| i.as_str())
        .map(str::to_string);
    let Some(envs) = value
        .get_mut("environments")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for env in envs {
        let Some(object) = env.as_object_mut() else {
            continue;
        };
        // 已是当前形状（base_url）则无需升级。
        if object.contains_key("base_url") {
            continue;
        }
        // 先算出升级结果（避免与 object 的可变借用冲突）。
        let (new_variables, base_url) =
            if let Some(map) = object.get("variables").and_then(|v| v.as_object()) {
                // v1：map 变量 → 结构化数组（base_url 键提升为基址）。
                let mut list: Vec<serde_json::Value> = Vec::new();
                let mut legacy_base_url: Option<String> = None;
                for (k, v) in map {
                    if k == "base_url" {
                        legacy_base_url = Some(
                            v.as_str()
                                .map(String::from)
                                .unwrap_or_else(|| v.to_string()),
                        );
                        continue;
                    }
                    if k.trim().is_empty() || k.starts_with("{{") || k.starts_with('$') {
                        continue;
                    }
                    list.push(serde_json::json!({
                    "key": k,
                    "remote_value": v.as_str().map(String::from).unwrap_or_else(|| v.to_string()),
                    "local_value": "",
                    "enabled": true,
                    "description": null,
                }));
                }
                (
                    Some(serde_json::Value::Array(list)),
                    legacy_base_url.unwrap_or_default(),
                )
            } else {
                // v2：多模块取默认 / 首个模块的基址。
                let base_url = object
                    .get("modules")
                    .and_then(|m| m.as_array())
                    .and_then(|ms| {
                        ms.iter()
                            .find(|m| {
                                m.get("is_default")
                                    .and_then(|d| d.as_bool())
                                    .unwrap_or(false)
                            })
                            .or_else(|| ms.first())
                    })
                    .and_then(|m| m.get("base_url"))
                    .and_then(|b| b.as_str())
                    .unwrap_or("")
                    .to_string();
                (None, base_url)
            };
        if let Some(vars) = new_variables {
            object.insert("variables".into(), vars);
        }
        object.remove("modules");
        object.insert("base_url".into(), serde_json::json!(base_url));
        if let Some(pid) = &project_id {
            object.insert("project_id".into(), serde_json::json!(pid));
        }
    }
}

/// 构建备份文件。
/// 备份输入（参数过多时收敛为结构体，避免“函数参数过多” lint）。
#[derive(Debug)]
pub struct BackupInput<'a> {
    pub project: &'a Project,
    pub folders: &'a [Folder],
    pub endpoints: &'a [Endpoint],
    pub environments: &'a [Environment],
    pub mock_rules: &'a [MockRule],
    pub response_examples: &'a [ResponseExample],
    pub request_examples: &'a [RequestExample],
    pub proto_files: &'a [ProtoFile],
    pub settings: &'a HashMap<String, String>,
    pub global_variables: &'a [EnvironmentVariable],
    pub global_params: &'a [GlobalParam],
    /// 团队元数据（v4）：项目所属团队，可空。
    pub team: &'a Option<Team>,
    /// 集成测试场景（v5）。
    pub scenarios: &'a [Scenario],
}

pub fn build_backup(input: &BackupInput) -> BackupFile {
    BackupFile {
        format: FORMAT.to_string(),
        schema_version: SCHEMA_VERSION,
        exported_at: Utc::now().to_rfc3339(),
        project: input.project.clone(),
        folders: input.folders.to_vec(),
        endpoints: input.endpoints.to_vec(),
        environments: input.environments.to_vec(),
        mock_rules: input.mock_rules.to_vec(),
        response_examples: input.response_examples.to_vec(),
        request_examples: input.request_examples.to_vec(),
        proto_files: input.proto_files.to_vec(),
        settings: input.settings.clone(),
        global_variables: input.global_variables.to_vec(),
        global_params: input.global_params.to_vec(),
        team: input.team.clone(),
        scenarios: input.scenarios.to_vec(),
    }
}

/// 恢复结果：所有实体均已重映射到新的 UUID，且原本的引用关系保持一致。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Restored {
    pub project: Project,
    pub folders: Vec<Folder>,
    pub endpoints: Vec<Endpoint>,
    pub environments: Vec<Environment>,
    pub mock_rules: Vec<MockRule>,
    pub response_examples: Vec<ResponseExample>,
    pub request_examples: Vec<RequestExample>,
    pub proto_files: Vec<ProtoFile>,
    pub scenarios: Vec<Scenario>,
}

/// 恢复：全量重映射 UUID（新项目）。返回值与 `build_backup` 顺序对应。
pub fn restore_backup(file: &BackupFile) -> Restored {
    let mut map: HashMap<Uuid, Uuid> = HashMap::new();

    let new_project_id = Uuid::new_v4();
    map.insert(file.project.id, new_project_id);
    let mut folders: Vec<Folder> = Vec::new();
    for f in &file.folders {
        let new_id = Uuid::new_v4();
        map.insert(f.id, new_id);
        folders.push(Folder {
            id: new_id,
            project_id: new_project_id,
            parent_id: f.parent_id.map(|p| *map.get(&p).unwrap_or(&new_project_id)),
            ..f.clone()
        });
    }

    let mut endpoints: Vec<Endpoint> = Vec::new();
    for e in &file.endpoints {
        let new_id = Uuid::new_v4();
        map.insert(e.id, new_id);
        endpoints.push(Endpoint {
            id: new_id,
            project_id: new_project_id,
            folder_id: e.folder_id.map(|p| *map.get(&p).unwrap_or(&new_id)),
            ..e.clone()
        });
    }

    let mut environments: Vec<Environment> = Vec::new();
    for e in &file.environments {
        environments.push(Environment {
            id: Uuid::new_v4(),
            project_id: new_project_id,
            ..e.clone()
        });
    }

    let mut mock_rules: Vec<MockRule> = Vec::new();
    for r in &file.mock_rules {
        mock_rules.push(MockRule {
            id: Uuid::new_v4(),
            project_id: new_project_id,
            endpoint_id: r.endpoint_id.map(|p| *map.get(&p).unwrap_or(&Uuid::nil())),
            ..r.clone()
        });
    }

    let mut response_examples: Vec<ResponseExample> = Vec::new();
    for e in &file.response_examples {
        response_examples.push(ResponseExample {
            id: Uuid::new_v4(),
            endpoint_id: *map.get(&e.endpoint_id).unwrap_or(&new_project_id),
            ..e.clone()
        });
    }

    let mut request_examples: Vec<RequestExample> = Vec::new();
    for e in &file.request_examples {
        request_examples.push(RequestExample {
            id: Uuid::new_v4(),
            endpoint_id: *map.get(&e.endpoint_id).unwrap_or(&new_project_id),
            ..e.clone()
        });
    }

    let mut proto_files: Vec<ProtoFile> = Vec::new();
    for f in &file.proto_files {
        proto_files.push(ProtoFile {
            id: Uuid::new_v4(),
            project_id: new_project_id,
            ..f.clone()
        });
    }

    // 场景：新 id 归新项目；步骤是接口引用式，endpoint_id 随接口 id 映射表
    // 重映射（备份内含全部接口，正常必然命中；手工编辑的残缺备份兜底 nil，
    // 与 mock_rules 的兜底口径一致）。
    let mut scenarios: Vec<Scenario> = Vec::with_capacity(file.scenarios.len());
    for s in &file.scenarios {
        let new_id = Uuid::new_v4();
        map.insert(s.id, new_id);
        let mut new_steps: Vec<miyo_core::model::ScenarioStep> = Vec::with_capacity(s.steps.len());
        for step in &s.steps {
            // 引用式步骤随接口 id 映射表重映射（备份内含全部接口，正常必然命中；
            // 手工编辑的残缺备份兜底 nil，与 mock_rules 的兜底口径一致）；
            // 内联步骤（endpoint_id None）无引用，原样保留。
            let mut mapped = step.clone();
            if let Some(old_ep) = step.endpoint_id {
                mapped.endpoint_id = Some(*map.get(&old_ep).unwrap_or(&Uuid::nil()));
            }
            new_steps.push(mapped);
        }
        scenarios.push(Scenario {
            id: new_id,
            project_id: new_project_id,
            steps: new_steps,
            ..s.clone()
        });
    }

    Restored {
        project: Project {
            id: new_project_id,
            ..file.project.clone()
        },
        folders,
        endpoints,
        environments,
        mock_rules,
        response_examples,
        request_examples,
        proto_files,
        scenarios,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{
        BodySpec, EndpointStatus, EnvironmentVariable, HttpMethod, KeyValue, RequestSpec,
        ScenarioStep,
    };
    fn sample_data() -> BackupFile {
        let project = Project {
            id: Uuid::new_v4(),
            name: "示例项目".into(),
            description: "desc".into(),
            variables: HashMap::from([("base_url".into(), "http://127.0.0.1".into())]),
            team_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let folder = Folder {
            id: Uuid::new_v4(),
            project_id: project.id,
            parent_id: None,
            name: "用户".into(),
            sort_order: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut request = RequestSpec::default();
        request.params.push(KeyValue::new("page", "1"));
        request.body = BodySpec::Json {
            raw: r#"{"a":1}"#.into(),
        };
        let ep = Endpoint {
            id: Uuid::new_v4(),
            project_id: project.id,
            folder_id: Some(folder.id),
            name: "列表".into(),
            method: HttpMethod::GET,
            path: "/users".into(),
            description: String::new(),
            status: EndpointStatus::Released,
            sort_order: 1,
            request,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let env = Environment {
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "测试".into(),
            base_url: "https://backup.example.com".into(),
            variables: vec![EnvironmentVariable {
                key: "token".into(),
                remote_value: "t1".into(),
                local_value: String::new(),
                enabled: true,
                local_override: None,
                sensitive: None,
                description: None,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let rule = MockRule {
            id: Uuid::new_v4(),
            project_id: project.id,
            endpoint_id: Some(ep.id),
            name: "规则".into(),
            method: HttpMethod::GET,
            path: "/users".into(),
            match_query: Vec::new(),
            match_headers: Vec::new(),
            response_status: 200,
            response_headers: HashMap::new(),
            response_body_template: "{}".into(),
            delay_ms: 0,
            fault_rate_pct: 0,
            fault_status: 500,
            enabled: true,
            priority: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let example = ResponseExample {
            id: Uuid::new_v4(),
            endpoint_id: ep.id,
            name: "成功".into(),
            status: 200,
            headers: HashMap::new(),
            body: r#"{"list":[]}"#.into(),
            content_type: "application/json".into(),
            docs: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut req_example_request = RequestSpec::default();
        req_example_request.params.push(KeyValue::new("page", "2"));
        let req_example = RequestExample {
            id: Uuid::new_v4(),
            endpoint_id: ep.id,
            name: "分页查询".into(),
            request: req_example_request,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let proto_file = ProtoFile {
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "user/v1/user.proto".into(),
            content: "syntax = \"proto3\";".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let scenario = Scenario {
            id: Uuid::new_v4(),
            project_id: project.id,
            name: "登录链路".into(),
            description: "登录 → 查详情".into(),
            category: String::new(),
            sort_order: 0,
            steps: vec![
                ScenarioStep {
                    endpoint_id: Some(ep.id),
                    enabled: true,
                    extract: serde_json::json!([{"var": "token", "from": "body", "path": "$.token"}]),
                    assertions: serde_json::json!([{"type": "status", "op": "eq", "value": 200}]),
                    name: None,
                    request: None,
                },
                // 指向不存在接口的步骤：恢复时应兜底 nil 而非 panic / 丢失。
                ScenarioStep {
                    endpoint_id: Some(Uuid::new_v4()),
                    enabled: false,
                    extract: serde_json::json!([]),
                    assertions: serde_json::json!([]),
                    name: None,
                    request: None,
                },
            ],
            script: Some("const steps = [];".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        build_backup(&BackupInput {
            project: &project,
            folders: &[folder],
            endpoints: &[ep],
            environments: &[env],
            mock_rules: &[rule],
            response_examples: &[example],
            request_examples: &[req_example],
            proto_files: &[proto_file],
            settings: &HashMap::from([("http_timeout_ms".into(), "30000".into())]),
            global_variables: &[],
            global_params: &[],
            team: &None,
            scenarios: &[scenario],
        })
    }

    #[test]
    fn serialize_parse_roundtrip() {
        let data = sample_data();
        let text = data.serialize().unwrap();
        let parsed = BackupFile::parse(&text).unwrap();
        assert_eq!(parsed, data);
    }

    #[test]
    fn parse_rejects_wrong_format() {
        assert!(BackupFile::parse(r#"{"format":"other"}"#).is_err());
    }

    #[test]
    fn parse_old_backup_without_request_examples_defaults_empty() {
        let mut data = sample_data();
        data.request_examples.clear();
        let text = data.serialize().unwrap();
        let parsed = BackupFile::parse(&text).unwrap();
        assert!(parsed.request_examples.is_empty());
        assert_eq!(parsed.response_examples.len(), 1);
    }

    /// 旧备份无 proto_files 字段时按空处理（不升 schema 版本）。
    #[test]
    fn parse_old_backup_without_proto_files_defaults_empty() {
        let data = sample_data();
        let mut v = serde_json::json!(data);
        v.as_object_mut().unwrap().remove("proto_files");
        let parsed = BackupFile::parse(&v.to_string()).unwrap();
        assert!(parsed.proto_files.is_empty());
    }

    /// v4 备份无 scenarios 字段：升级到 v5 后按空处理，场景数据不阻塞导入。
    #[test]
    fn parse_v4_backup_without_scenarios_defaults_empty() {
        let data = sample_data();
        let mut v4 = serde_json::json!(data);
        v4["schema_version"] = serde_json::json!(4);
        v4.as_object_mut().unwrap().remove("scenarios");
        let parsed = BackupFile::parse(&v4.to_string()).unwrap();
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
        assert!(parsed.scenarios.is_empty());
        // 其余实体不受影响。
        assert_eq!(parsed.endpoints.len(), 1);
    }

    #[test]
    fn new_settings_fields_roundtrip_and_old_backup_defaults_empty() {
        let data = sample_data();
        assert_eq!(
            data.settings.get("http_timeout_ms").map(String::as_str),
            Some("30000")
        );
        let text = data.serialize().unwrap();
        let parsed = BackupFile::parse(&text).unwrap();
        assert_eq!(parsed.settings, data.settings);
        // 旧备份（无新字段）按空处理。
        let mut v = serde_json::json!(data);
        for key in ["settings", "global_variables", "global_params"] {
            v.as_object_mut().unwrap().remove(key);
        }
        let old = BackupFile::parse(&v.to_string()).unwrap();
        assert!(old.settings.is_empty());
        assert!(old.global_variables.is_empty());
        assert!(old.global_params.is_empty());
    }

    #[test]
    fn parse_v1_backup_upgrades_env_variables() {
        // 构造 v3 备份 → 还原为 v1 形状（map 变量、无 base_url / project_id）。
        let data = sample_data();
        let mut v1 = serde_json::json!(data);
        v1["schema_version"] = serde_json::json!(1);
        let env0 = &mut v1["environments"][0];
        env0.as_object_mut().unwrap().remove("base_url");
        env0.as_object_mut().unwrap().remove("project_id");
        env0["variables"] = serde_json::json!({
            "base_url": "https://legacy.example.com",
            "token": "t1",
        });
        let parsed = BackupFile::parse(&v1.to_string()).unwrap();
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
        let env = &parsed.environments[0];
        // base_url 键 → 环境 Base URL
        assert_eq!(env.base_url, "https://legacy.example.com");
        // project_id 占位为文件内项目
        assert_eq!(env.project_id, data.project.id);
        // 其余键 → 结构化变量
        assert_eq!(env.variables.len(), 1);
        assert_eq!(env.variables[0].key, "token");
        assert_eq!(env.variables[0].remote_value, "t1");
        assert!(env.variables[0].enabled);
    }

    /// v2（多模块）备份升级：默认模块的基址提升为环境 Base URL。
    #[test]
    fn parse_v2_backup_upgrades_modules_to_base_url() {
        let data = sample_data();
        let mut v2 = serde_json::json!(data);
        v2["schema_version"] = serde_json::json!(2);
        let env0 = &mut v2["environments"][0];
        env0.as_object_mut().unwrap().remove("base_url");
        env0["modules"] = serde_json::json!([
            { "id": Uuid::new_v4().to_string(), "module_name": "支付", "base_url": "https://pay.example.com", "is_default": true },
            { "id": Uuid::new_v4().to_string(), "module_name": "收单", "base_url": "https://acq.example.com", "is_default": false },
        ]);
        let parsed = BackupFile::parse(&v2.to_string()).unwrap();
        let env = &parsed.environments[0];
        assert_eq!(env.base_url, "https://pay.example.com");
        assert_eq!(env.project_id, data.project.id);
    }

    /// v3（无团队信息）备份升级到 v4：team 缺失 → None，project.team_id 缺失 → None。
    #[test]
    fn parse_v3_backup_without_team_metadata() {
        let data = sample_data();
        let mut v3 = serde_json::json!(data);
        v3["schema_version"] = serde_json::json!(3);
        v3.as_object_mut().unwrap().remove("team");
        v3["project"].as_object_mut().unwrap().remove("team_id");
        let parsed = BackupFile::parse(&v3.to_string()).unwrap();
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
        assert!(parsed.team.is_none());
        assert!(parsed.project.team_id.is_none());
    }

    #[test]
    fn restore_remaps_all_ids_consistently() {
        let data = sample_data();
        let restored = restore_backup(&data);
        // 新的项目 id
        assert_ne!(restored.project.id, data.project.id);
        assert_eq!(restored.project.name, data.project.name);
        assert_eq!(restored.project.variables, data.project.variables);
        // 文件夹引用新项目
        assert_eq!(restored.folders.len(), 1);
        let f = &restored.folders[0];
        assert_eq!(f.project_id, restored.project.id);
        assert_ne!(f.id, data.folders[0].id);
        // 接口引用新区块
        let ep = &restored.endpoints[0];
        assert_eq!(ep.project_id, restored.project.id);
        assert_eq!(ep.folder_id, Some(f.id));
        assert_eq!(ep.method, HttpMethod::GET);
        assert_eq!(ep.request.params.len(), 1);
        // MockRule 与 ResponseExample 引用新的 endpoint id
        assert_eq!(restored.mock_rules[0].endpoint_id, Some(ep.id));
        assert_eq!(restored.response_examples[0].endpoint_id, ep.id);
        // 环境重映射归属到新项目，基址保留
        assert_eq!(restored.environments[0].project_id, restored.project.id);
        assert_eq!(
            restored.environments[0].base_url,
            "https://backup.example.com"
        );
        // 请求用例：引用新 endpoint id、请求快照保持、名称一致
        let req_ex = &restored.request_examples[0];
        assert_eq!(req_ex.endpoint_id, ep.id);
        assert_eq!(req_ex.name, "分页查询");
        assert_eq!(req_ex.request.params[0].key, "page");
        assert_ne!(req_ex.id, data.request_examples[0].id);
        // proto 文件：重映射归属新项目，内容保留
        assert_eq!(restored.proto_files.len(), 1);
        assert_eq!(restored.proto_files[0].project_id, restored.project.id);
        assert_eq!(restored.proto_files[0].name, "user/v1/user.proto");
        assert_ne!(restored.proto_files[0].id, data.proto_files[0].id);
        // 场景：新 id 归新项目；步骤 endpoint_id 跟随接口重映射；
        // 未命中映射的步骤兜底 nil（不丢失、不指向旧 id）。
        let sc = &restored.scenarios[0];
        assert_eq!(sc.project_id, restored.project.id);
        assert_ne!(sc.id, data.scenarios[0].id);
        assert_eq!(sc.steps[0].endpoint_id, Some(ep.id));
        assert_eq!(sc.steps[1].endpoint_id, Some(Uuid::nil()));
        assert_eq!(sc.steps[0].extract, data.scenarios[0].steps[0].extract);
        assert_eq!(sc.script, data.scenarios[0].script);
        // 无交叉引用残留
        let old_ids: Vec<Uuid> = data.endpoints.iter().map(|e| e.id).collect();
        for new_ep in &restored.endpoints {
            assert!(!old_ids.contains(&new_ep.id));
        }
    }

    #[test]
    fn restore_is_idempotent_shape() {
        let data = sample_data();
        let a = restore_backup(&data);
        let b = restore_backup(&data);
        assert_ne!(a.project.id, b.project.id);
        assert_eq!(a.endpoints[0].name, b.endpoints[0].name);
    }
}
