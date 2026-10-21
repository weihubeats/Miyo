//! SKILL.md 的 frontmatter 解析与序列化。
//!
//! 格式（各 agent 工具通用）：
//!
//! ```markdown
//! ---
//! name: my-skill
//! description: 一句话说明何时使用
//! ---
//!
//! 正文…
//! ```
//!
//! 解析是**宽松**的：缺 frontmatter / YAML 非法时降级为「目录名 + 空描述 + 告警」，
//! 保证漂移文件仍能出现在列表里被修复，而不是整个面板报错。

use std::collections::BTreeMap;

use crate::error::SkillError;
use crate::model::SkillFrontmatter;

/// 宽松解析结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub frontmatter: SkillFrontmatter,
    /// 正文（已剥离 frontmatter）。
    pub body: String,
    /// 解析降级原因（用于 UI 告警）。
    pub warning: Option<String>,
}

/// 宽松解析：永不失败。
pub fn parse_lenient(raw: &str, fallback_name: &str) -> Parsed {
    let (frontmatter, body, warning) = match split(raw) {
        Ok((yaml, body)) => match parse_yaml(&yaml) {
            Ok(frontmatter) => {
                let warning = if frontmatter.description.trim().is_empty() {
                    Some("description 为空，部分工具不会加载该 skill".to_string())
                } else {
                    None
                };
                (frontmatter, body, warning)
            }
            Err(err) => (
                SkillFrontmatter {
                    name: fallback_name.to_string(),
                    ..Default::default()
                },
                body,
                Some(err.to_string()),
            ),
        },
        Err(err) => (
            SkillFrontmatter {
                name: fallback_name.to_string(),
                ..Default::default()
            },
            raw.trim_start_matches(['\n', '\r']).to_string(),
            Some(err.to_string()),
        ),
    };

    let mut frontmatter = frontmatter;
    if frontmatter.name.trim().is_empty() {
        frontmatter.name = fallback_name.to_string();
    }

    Parsed {
        frontmatter,
        body,
        warning,
    }
}

/// 严格解析（保存 / 导入时校验）。
pub fn parse_strict(raw: &str) -> Result<(SkillFrontmatter, String), SkillError> {
    let (yaml, body) = split(raw)?;
    Ok((parse_yaml(&yaml)?, body))
}

/// 序列化为标准 SKILL.md 文本。
pub fn render(frontmatter: &SkillFrontmatter, body: &str) -> Result<String, SkillError> {
    let mut map = serde_json::Map::new();
    map.insert(
        "name".to_string(),
        serde_json::Value::String(frontmatter.name.trim().to_string()),
    );
    map.insert(
        "description".to_string(),
        serde_json::Value::String(frontmatter.description.trim().to_string()),
    );
    for (key, value) in &frontmatter.extra {
        let key = key.trim();
        if key.is_empty() || key == "name" || key == "description" {
            continue;
        }
        map.insert(key.to_string(), value.clone());
    }

    let yaml = serde_norway::to_string(&serde_json::Value::Object(map))
        .map_err(|err| SkillError::Frontmatter(format!("YAML 序列化失败：{err}")))?;

    let body = body.trim_start_matches(['\n', '\r']);
    let mut out = String::with_capacity(yaml.len() + body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n\n");
    out.push_str(body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// 拆出 frontmatter 与正文；缺失或未闭合时返回错误。
fn split(raw: &str) -> Result<(String, String), SkillError> {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);

    let rest = text
        .strip_prefix("---")
        .and_then(|tail| {
            tail.strip_prefix("\r\n")
                .or_else(|| tail.strip_prefix('\n'))
        })
        .ok_or_else(|| SkillError::Frontmatter("缺少 frontmatter（文件需以 `---` 开头）".into()))?;

    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if trimmed == "---" || trimmed == "..." {
            let yaml = rest[..offset].to_string();
            let body = rest[offset + line.len()..].to_string();
            return Ok((yaml, body));
        }
        offset += line.len();
    }

    Err(SkillError::Frontmatter(
        "frontmatter 未闭合（缺少结束行 `---`）".into(),
    ))
}

fn parse_yaml(yaml: &str) -> Result<SkillFrontmatter, SkillError> {
    let value: serde_norway::Value = serde_norway::from_str(yaml)
        .map_err(|err| SkillError::Frontmatter(format!("YAML 解析失败：{err}")))?;

    let mapping = match value {
        serde_norway::Value::Mapping(map) => map,
        serde_norway::Value::Null => Default::default(),
        _ => return Err(SkillError::Frontmatter("frontmatter 必须是键值映射".into())),
    };

    let mut frontmatter = SkillFrontmatter::default();
    for (key, value) in mapping {
        let key = match key {
            serde_norway::Value::String(key) => key,
            other => {
                return Err(SkillError::Frontmatter(format!(
                    "frontmatter 键必须是字符串：{other:?}"
                )))
            }
        };
        let json = serde_json::to_value(&value)
            .map_err(|err| SkillError::Frontmatter(format!("字段 {key} 转换失败：{err}")))?;
        match key.as_str() {
            "name" => {
                frontmatter.name = json.as_str().unwrap_or_default().to_string();
            }
            "description" => {
                frontmatter.description = json.as_str().unwrap_or_default().to_string();
            }
            _ => {
                frontmatter.extra.insert(key, json);
            }
        }
    }
    Ok(frontmatter)
}

/// 空 skill 模板（新建时兜底）。
pub fn template(name: &str) -> SkillFrontmatter {
    SkillFrontmatter {
        name: name.to_string(),
        description: String::new(),
        extra: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_keeps_extra() {
        let raw = "---\nname: demo\ndescription: 演示\nallowed-tools: Read\n---\n\n正文\n";
        let parsed = parse_lenient(raw, "fallback");
        assert_eq!(parsed.frontmatter.name, "demo");
        assert_eq!(parsed.frontmatter.description, "演示");
        assert_eq!(
            parsed.frontmatter.extra.get("allowed-tools"),
            Some(&serde_json::Value::String("Read".into()))
        );
        assert_eq!(parsed.body.trim(), "正文");
        assert!(parsed.warning.is_none());
    }

    #[test]
    fn missing_frontmatter_degrades_to_fallback() {
        let parsed = parse_lenient("# 只有正文\n", "dir-name");
        assert_eq!(parsed.frontmatter.name, "dir-name");
        assert!(parsed.body.contains("只有正文"));
        assert!(parsed.warning.is_some());
    }

    #[test]
    fn unclosed_frontmatter_warns() {
        let parsed = parse_lenient("---\nname: x\n", "x");
        assert_eq!(parsed.frontmatter.name, "x");
        assert!(parsed.warning.unwrap().contains("未闭合"));
    }

    #[test]
    fn render_round_trips() {
        let raw = "---\nname: demo\ndescription: \"含: 冒号\"\n---\n\n正文\n";
        let (fm, body) = parse_strict(raw).unwrap();
        let rendered = render(&fm, &body).unwrap();
        let (fm2, body2) = parse_strict(&rendered).unwrap();
        assert_eq!(fm, fm2);
        assert_eq!(body.trim(), body2.trim());
    }

    #[test]
    fn crlf_is_supported() {
        let raw = "---\r\nname: win\r\ndescription: d\r\n---\r\n\r\nbody\r\n";
        let (fm, _) = parse_strict(raw).unwrap();
        assert_eq!(fm.name, "win");
    }
}
