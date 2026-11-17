//! OpenAPI 3.0 导出（M7 / M10）。

use std::collections::HashMap;

use indexmap::IndexMap;
use miyo_core::model::{BodySpec, Endpoint, HttpMethod, KeyValue, ResponseExample};
use miyo_core::AppError;
use openapiv3::{
    Info, MediaType, OpenAPI, Operation, Parameter, ParameterData, ParameterSchemaOrContent,
    PathItem, Paths, ReferenceOr, RequestBody, Response, Responses, Schema, SchemaData, SchemaKind,
    StatusCode, StringType, Type,
};

fn string_schema() -> ReferenceOr<Schema> {
    ReferenceOr::Item(Schema {
        schema_data: SchemaData::default(),
        schema_kind: SchemaKind::Type(Type::String(StringType::default())),
    })
}

fn kv_to_parameter(kv: &KeyValue) -> Option<Parameter> {
    let example = if kv.value.is_empty() {
        None
    } else {
        Some(serde_json::Value::String(kv.value.clone()))
    };
    let data = ParameterData {
        name: kv.key.clone(),
        description: if kv.description.is_empty() {
            None
        } else {
            Some(kv.description.clone())
        },
        required: false,
        deprecated: None,
        format: ParameterSchemaOrContent::Schema(string_schema()),
        example,
        examples: IndexMap::new(),
        explode: None,
        extensions: IndexMap::new(),
    };
    let _ = kv; // enabled 在 OpenAPI 中无对应字段，忽略
    Some(Parameter::Query {
        parameter_data: data,
        allow_reserved: false,
        style: Default::default(),
        allow_empty_value: None,
    })
}

/// 将一组接口导出为 OpenAPI 3.0 JSON 文本。
/// `examples_by_endpoint`：endpoint_id -> 响应示例列表（可为空）。
pub fn export_project(
    project_name: &str,
    endpoints: &[Endpoint],
    examples_by_endpoint: &HashMap<uuid::Uuid, Vec<ResponseExample>>,
) -> Result<String, AppError> {
    let value = export_project_value(project_name, endpoints, examples_by_endpoint)?;
    serde_json::to_string_pretty(&value).map_err(|e| AppError::OpenApi(format!("导出失败：{e}")))
}

/// 将一组接口导出为 OpenAPI 结构值（JSON/YAML 双格式共用，避免
/// 「结构 → JSON 文本 → Value → YAML」的双重编解码）。
pub fn export_project_value(
    project_name: &str,
    endpoints: &[Endpoint],
    examples_by_endpoint: &HashMap<uuid::Uuid, Vec<ResponseExample>>,
) -> Result<serde_json::Value, AppError> {
    let mut path_map: IndexMap<String, PathItem> = IndexMap::new();

    for ep in endpoints {
        // OpenAPI 是 HTTP 规范：gRPC / Java 方法端点不参与导出
        if ep.method.is_grpc() || ep.method.is_java() {
            continue;
        }
        let path_item = path_map.entry(ep.path.clone()).or_default();
        let op = build_operation(ep, examples_by_endpoint.get(&ep.id));
        match ep.method {
            HttpMethod::GET => path_item.get = Some(op),
            HttpMethod::POST => path_item.post = Some(op),
            HttpMethod::PUT => path_item.put = Some(op),
            HttpMethod::DELETE => path_item.delete = Some(op),
            HttpMethod::PATCH => path_item.patch = Some(op),
            HttpMethod::HEAD => path_item.head = Some(op),
            HttpMethod::OPTIONS => path_item.options = Some(op),
            HttpMethod::Grpc => unreachable!("gRPC 端点已在上方跳过"),
            HttpMethod::Java => unreachable!("Java 端点已在上方跳过"),
        }
    }

    let spec = OpenAPI {
        openapi: "3.0.3".into(),
        info: Info {
            title: project_name.to_string(),
            description: None,
            terms_of_service: None,
            contact: None,
            license: None,
            version: "1.0.0".into(),
            extensions: IndexMap::new(),
        },
        servers: Vec::new(),
        paths: Paths {
            paths: path_map
                .into_iter()
                .map(|(k, v)| (k, ReferenceOr::Item(v)))
                .collect(),
            extensions: IndexMap::new(),
        },
        components: None,
        security: None,
        tags: Vec::new(),
        external_docs: None,
        extensions: IndexMap::new(),
    };

    serde_json::to_value(&spec).map_err(|e| AppError::OpenApi(format!("导出失败：{e}")))
}

fn build_operation(ep: &Endpoint, examples: Option<&Vec<ResponseExample>>) -> Operation {
    let mut parameters = Vec::new();
    for kv in &ep.request.params {
        if kv.enabled {
            if let Some(p) = kv_to_parameter(kv) {
                parameters.push(ReferenceOr::Item(p));
            }
        }
    }
    for kv in &ep.request.headers {
        if kv.enabled {
            parameters.push(ReferenceOr::Item(Parameter::Header {
                parameter_data: parameter_data_from_kv(kv),
                style: Default::default(),
            }));
        }
    }
    for kv in &ep.request.path_variables {
        if kv.enabled {
            parameters.push(ReferenceOr::Item(Parameter::Path {
                parameter_data: parameter_data_from_kv(kv),
                style: Default::default(),
            }));
        }
    }

    let request_body = match &ep.request.body {
        BodySpec::Json { raw } => {
            let example = serde_json::from_str(raw)
                .unwrap_or_else(|_| serde_json::Value::String(raw.clone()));
            let mut content = IndexMap::new();
            content.insert(
                "application/json".into(),
                MediaType {
                    schema: None,
                    example: Some(example),
                    examples: IndexMap::new(),
                    encoding: IndexMap::new(),
                    extensions: IndexMap::new(),
                },
            );
            Some(ReferenceOr::Item(RequestBody {
                description: None,
                content,
                required: false,
                extensions: IndexMap::new(),
            }))
        }
        _ => None,
    };

    let mut responses = IndexMap::new();
    match examples {
        Some(list) => {
            for ex in list {
                let status = StatusCode::Code(ex.status);
                let mut content = IndexMap::new();
                let media = MediaType {
                    schema: None,
                    example: parse_example_body(&ex.body),
                    examples: IndexMap::new(),
                    encoding: IndexMap::new(),
                    extensions: IndexMap::new(),
                };
                content.insert(ex.content_type.clone(), media);
                responses.insert(
                    status,
                    ReferenceOr::Item(Response {
                        description: ex.name.clone(),
                        headers: IndexMap::new(),
                        content,
                        links: IndexMap::new(),
                        extensions: IndexMap::new(),
                    }),
                );
            }
        }
        None => {
            let mut content = IndexMap::new();
            content.insert(
                "application/json".into(),
                MediaType {
                    schema: None,
                    example: Some(serde_json::Value::Object(Default::default())),
                    examples: IndexMap::new(),
                    encoding: IndexMap::new(),
                    extensions: IndexMap::new(),
                },
            );
            responses.insert(
                StatusCode::Code(200),
                ReferenceOr::Item(Response {
                    description: "成功".into(),
                    headers: IndexMap::new(),
                    content,
                    links: IndexMap::new(),
                    extensions: IndexMap::new(),
                }),
            );
        }
    }

    Operation {
        tags: Vec::new(),
        summary: Some(ep.name.clone()),
        description: if ep.description.is_empty() {
            None
        } else {
            Some(ep.description.clone())
        },
        external_docs: None,
        operation_id: None,
        parameters,
        request_body,
        responses: Responses {
            default: None,
            responses,
            extensions: IndexMap::new(),
        },
        callbacks: IndexMap::new(),
        deprecated: false,
        security: None,
        servers: Vec::new(),
        extensions: IndexMap::new(),
    }
}

fn parameter_data_from_kv(kv: &KeyValue) -> ParameterData {
    ParameterData {
        name: kv.key.clone(),
        description: if kv.description.is_empty() {
            None
        } else {
            Some(kv.description.clone())
        },
        required: false,
        deprecated: None,
        format: ParameterSchemaOrContent::Schema(string_schema()),
        example: if kv.value.is_empty() {
            None
        } else {
            Some(serde_json::Value::String(kv.value.clone()))
        },
        examples: IndexMap::new(),
        explode: None,
        extensions: IndexMap::new(),
    }
}

fn parse_example_body(body: &str) -> Option<serde_json::Value> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed)
        .ok()
        .or_else(|| Some(serde_json::Value::String(body.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::import_endpoints;
    use chrono::Utc;
    use miyo_core::model::{EndpointStatus, RequestSpec};

    fn sample_endpoint() -> Endpoint {
        Endpoint {
            id: uuid::Uuid::new_v4(),
            project_id: uuid::Uuid::new_v4(),
            folder_id: None,
            name: "用户列表".into(),
            method: HttpMethod::GET,
            path: "/users".into(),
            description: "获取用户".into(),
            status: EndpointStatus::Developing,
            sort_order: 0,
            request: RequestSpec {
                params: vec![KeyValue::new("page", "1")],
                headers: vec![],
                path_variables: vec![],
                auth: Default::default(),
                body: BodySpec::None,
                ..Default::default()
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn export_basic_document() {
        let eps = vec![sample_endpoint()];
        let json = export_project("Demo", &eps, &HashMap::new()).unwrap();
        let parsed: OpenAPI = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.openapi, "3.0.3");
        assert_eq!(parsed.info.title, "Demo");
        assert_eq!(parsed.paths.paths.len(), 1);
        let item = parsed.paths.paths.get("/users").unwrap();
        let ReferenceOr::Item(item) = item else {
            panic!("path item should be inline")
        };
        let op = item.get.as_ref().expect("get operation");
        assert_eq!(op.summary.as_deref(), Some("用户列表"));
        assert_eq!(op.parameters.len(), 1);
        assert_eq!(op.responses.responses.len(), 1);
        assert!(op.responses.responses.contains_key(&StatusCode::Code(200)));
    }

    #[test]
    fn export_roundtrip_import() {
        let eps = vec![sample_endpoint()];
        let json = export_project("Demo", &eps, &HashMap::new()).unwrap();
        let reimported = import_endpoints(&json).unwrap();
        assert_eq!(reimported.len(), 1);
        assert_eq!(reimported[0].path, "/users");
        assert_eq!(reimported[0].method, HttpMethod::GET);
        assert_eq!(reimported[0].request.params[0].key, "page");
    }

    #[test]
    fn export_json_body_and_examples() {
        let mut ep = sample_endpoint();
        ep.request.body = BodySpec::Json {
            raw: r#"{"name":"张三"}"#.into(),
        };
        ep.method = HttpMethod::POST;
        let ex = ResponseExample {
            id: uuid::Uuid::new_v4(),
            endpoint_id: ep.id,
            name: "200 application/json".into(),
            status: 200,
            headers: HashMap::new(),
            body: r#"{"ok":true}"#.into(),
            content_type: "application/json".into(),
            docs: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut map = HashMap::new();
        map.insert(ep.id, vec![ex]);
        let json = export_project("Demo", &[ep], &map).unwrap();
        let reimported = import_endpoints(&json).unwrap();
        assert_eq!(reimported.len(), 1);
        let got = &reimported[0];
        assert!(matches!(got.request.body, BodySpec::Json { .. }));
        assert_eq!(got.examples.len(), 1);
        assert_eq!(got.examples[0].status, 200);
        assert!(got.examples[0].body.contains("ok"));
    }
}
