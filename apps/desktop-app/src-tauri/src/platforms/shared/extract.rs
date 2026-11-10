//! 响应解析的通用小工具。
//!
//! 各平台的成功响应结构差异极大（`code` / `err_no` / `errcode` / 纯 `data`），
//! 但「把文章 ID 从 JSON 里挖出来」「把业务码读成整数」这两件事是同构的。
//! CSDN 与掘金早期各自复制了一份实现，新平台统一走这里，避免第六份拷贝。

use serde_json::Value;

/// 平台常返回 `0` / `""` / `(未知)` 这类占位 ID，它们不能当作真实文章 ID 写进同步映射。
pub fn is_placeholder_id(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    trimmed.is_empty() || trimmed == "0" || trimmed == "(未知)" || trimmed == "未知"
}

fn value_to_id(value: &Value) -> Option<String> {
    let candidate = if let Some(s) = value.as_str() {
        s.trim().to_string()
    } else if let Some(n) = value.as_i64() {
        n.to_string()
    } else if let Some(n) = value.as_u64() {
        n.to_string()
    } else {
        return None;
    };

    if is_placeholder_id(&candidate) {
        None
    } else {
        Some(candidate)
    }
}

/// 按 `keys` 顺序在顶层查找 ID，找不到再下探一层 `data` / `draft` / `article`。
///
/// 只下探一层是刻意的：更深的递归会误命中列表里其它文章的 ID，
/// 那比「提取失败」危险得多。
pub fn extract_id(data: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = data.get(*key) {
            if let Some(id) = value_to_id(v) {
                return Some(id);
            }
        }
    }

    for nested in ["data", "draft", "article", "note"] {
        if let Some(inner) = data.get(nested) {
            for key in keys {
                if let Some(v) = inner.get(*key) {
                    if let Some(id) = value_to_id(v) {
                        return Some(id);
                    }
                }
            }
        }
    }

    None
}

/// 读取业务码。不同平台字段名不同，因此传入候选字段名列表，取第一个能解析成整数的。
pub fn read_numeric_field(response: &Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        match response.get(*key) {
            Some(Value::Number(n)) => {
                if let Some(v) = n.as_i64() {
                    return Some(v);
                }
            }
            Some(Value::String(s)) => {
                if let Ok(v) = s.trim().parse::<i64>() {
                    return Some(v);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_string_and_numeric_ids() {
        assert_eq!(
            extract_id(&json!({ "id": "abc123" }), &["id"]),
            Some("abc123".to_string())
        );
        assert_eq!(
            extract_id(&json!({ "id": 987 }), &["id"]),
            Some("987".to_string())
        );
    }

    #[test]
    fn treats_placeholder_ids_as_missing() {
        assert_eq!(extract_id(&json!({ "id": "0" }), &["id"]), None);
        assert_eq!(extract_id(&json!({ "id": "" }), &["id"]), None);
        assert_eq!(extract_id(&json!({ "id": "(未知)" }), &["id"]), None);
    }

    #[test]
    fn descends_exactly_one_level() {
        assert_eq!(
            extract_id(&json!({ "data": { "article_id": "42" } }), &["article_id"]),
            Some("42".to_string())
        );
        assert_eq!(
            extract_id(&json!({ "data": { "draft": { "id": "42" } } }), &["id"]),
            None,
            "只下探一层，避免误命中深层列表里的其它 ID"
        );
    }

    #[test]
    fn reads_numeric_field_from_number_or_string() {
        assert_eq!(read_numeric_field(&json!({ "code": 200 }), &["code"]), Some(200));
        assert_eq!(
            read_numeric_field(&json!({ "code": "0" }), &["code"]),
            Some(0)
        );
        assert_eq!(
            read_numeric_field(&json!({ "errcode": 1 }), &["code", "errcode"]),
            Some(1)
        );
        assert_eq!(read_numeric_field(&json!({}), &["code"]), None);
    }
}