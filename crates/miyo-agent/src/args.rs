//! MCP 工具入参结构体（单源试点）。
//!
//! 「结构体 ↔ schema ↔ dispatch 解析」三处并存是漂移面：本模块把试点工具的
//! 入参收敛为 typed struct——`derive(JsonSchema)` 生成 inputSchema（doc 注释
//! 即属性 description），dispatch 用 `serde_json::from_value` 反序列化，字段
//! 名经 `rename_all = "camelCase"` 与 MCP wire 契约一致。
//!
//! 渐进迁移路径：新增工具优先在此定义参数结构体并在 [`crate::registry`]
//! 的 GENERATED_TOOL_SCHEMAS 注册；含丰富调用示例的复杂工具（如
//! run_scenario 的 steps 说明）可暂留 TOOL_SCHEMA 手写表。

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;

/// `list_endpoints` 入参。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListEndpointsArgs {
    /// 项目 UUID（先调 list_projects 获取）
    pub project_id: String,
    /// 可选，名称/路径子串过滤（大小写不敏感）
    pub name: Option<String>,
    /// 可选，HTTP 方法过滤（GET/POST/…，大小写不敏感）
    pub method: Option<String>,
    /// 可选，分组 UUID 过滤
    pub folder_id: Option<String>,
    /// 可选，分页条数（配合 offset；缺省全量）
    pub limit: Option<u64>,
    /// 可选，分页偏移（缺省 0）
    pub offset: Option<u64>,
}

/// `get_endpoint` 入参。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetEndpointArgs {
    /// 接口 UUID（list_endpoints / save_curl 返回）
    pub endpoint_id: String,
}

/// 试点工具名 → 生成 inputSchema（serde_json Value，type=object）。
pub fn generated_tool_schemas() -> &'static BTreeMap<&'static str, serde_json::Value> {
    use std::sync::OnceLock;
    static MAP: OnceLock<BTreeMap<&'static str, serde_json::Value>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = BTreeMap::new();
        m.insert(
            "list_endpoints",
            serde_json::to_value(schemars::schema_for!(ListEndpointsArgs))
                .expect("list_endpoints schema"),
        );
        m.insert(
            "get_endpoint",
            serde_json::to_value(schemars::schema_for!(GetEndpointArgs))
                .expect("get_endpoint schema"),
        );
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 生成 schema 与手写契约逐字段对齐：type=object、required、camelCase 属性名。
    #[test]
    fn list_endpoints_schema_matches_wire_contract() {
        let s = generated_tool_schemas()["list_endpoints"].clone();
        assert_eq!(s["type"], "object");
        assert_eq!(s["required"], json!(["projectId"]));
        for key in ["projectId", "name", "method", "folderId", "limit", "offset"] {
            assert!(s["properties"].get(key).is_some(), "缺属性 {key}");
        }
        // doc 注释 → description（LLM 文档随结构体单源）
        assert!(s["properties"]["projectId"]["description"]
            .as_str()
            .unwrap()
            .contains("项目 UUID"));
    }

    #[test]
    fn get_endpoint_schema_matches_wire_contract() {
        let s = generated_tool_schemas()["get_endpoint"].clone();
        assert_eq!(s["type"], "object");
        assert_eq!(s["required"], json!(["endpointId"]));
        assert!(s["properties"]["endpointId"].is_object());
    }

    /// 结构体反序列化与 MCP wire 键一致（camelCase）。
    #[test]
    fn args_deserialize_from_camel_case_payload() {
        let args: ListEndpointsArgs = serde_json::from_value(json!({
            "projectId": "00000000-0000-0000-0000-000000000001",
            "limit": 10,
        }))
        .unwrap();
        assert_eq!(args.project_id, "00000000-0000-0000-0000-000000000001");
        assert_eq!(args.limit, Some(10));
        assert_eq!(args.name, None);
    }
}
