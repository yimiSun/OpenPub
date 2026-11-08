pub const MISSING_KEY_PREFIX: &str = "Cookie 中缺少 ";

use tauri_plugin_store::StoreExt;

use crate::platforms::FailureClass;

pub fn load_from_store(
    app: &tauri::AppHandle,
    config_key: &str,
    platform: &str,
) -> Result<String, (FailureClass, String)> {
    let store = app
        .store("platform_config.json")
        .map_err(|e| (FailureClass::NotConfigured, format!("读取配置失败: {}", e)))?;

    let manual = store
        .get(config_key)
        .map(|config| {
            config
                .get("cookie")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default();

    if !manual.is_empty() {
        return Ok(manual);
    }

    match crate::load_stored_cookie(app, platform) {
        Ok(Some(stored)) if !stored.trim().is_empty() => return Ok(stored),
        Ok(_) => {}
        Err(message) => return Err((FailureClass::NotConfigured, message)),
    }

    Err((
        FailureClass::NotConfigured,
        format!("{}未配置，请先登录或填写 Cookie", platform),
    ))
}

/// 读取 `platform_config.json` 中 `{config_key}` 对象里的单个字段（非 Cookie 字段）。
/// 微信的 token 放在配置对象里而不是 Cookie 里，`load_from_store` 只能取 cookie，
/// 因此需要这个入口读取 token。
pub fn load_config_field(
    app: &tauri::AppHandle,
    config_key: &str,
    field: &str,
) -> Result<Option<String>, String> {
    let store = app
        .store("platform_config.json")
        .map_err(|e| format!("读取配置失败: {}", e))?;

    let config = match store.get(config_key) {
        Some(value) => value,
        None => return Ok(None),
    };

    let raw = config
        .get(field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    Ok(if raw.is_empty() { None } else { Some(raw) })
}

pub fn extract_value(cookie: &str, key: &str) -> Option<String> {
    let mut found: Option<String> = None;

    for segment in cookie.split(';') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (name, value) = match trimmed.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        if name.trim() == key {
            found = Some(value.trim().to_string());
        }
    }

    found.filter(|value| !value.is_empty())
}

pub fn require_value(cookie: &str, key: &str, platform: &str) -> Result<String, String> {
    extract_value(cookie, key).ok_or_else(|| {
        format!(
            "{}{}，请重新登录 {} 后复制完整 Cookie",
            MISSING_KEY_PREFIX, key, platform
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_named_key() {
        let cookie = "a=1; _xsrf=TOKEN; b=2";
        assert_eq!(extract_value(cookie, "_xsrf"), Some("TOKEN".to_string()));
        assert_eq!(extract_value(cookie, "a"), Some("1".to_string()));
    }

    #[test]
    fn tolerates_missing_space_after_separator() {
        let cookie = "a=1;_xsrf=TOKEN;b=2";
        assert_eq!(extract_value(cookie, "_xsrf"), Some("TOKEN".to_string()));
    }

    #[test]
    fn last_occurrence_wins_for_duplicate_keys() {
        let cookie = "_xsrf=OLD; _xsrf=NEW";
        assert_eq!(extract_value(cookie, "_xsrf"), Some("NEW".to_string()));
    }

    #[test]
    fn skips_segments_without_equals_sign() {
        let cookie = "flag; _xsrf=TOKEN; junk";
        assert_eq!(extract_value(cookie, "_xsrf"), Some("TOKEN".to_string()));
    }

    #[test]
    fn missing_key_returns_none_not_blank() {
        let cookie = "a=1; b=2";
        assert_eq!(extract_value(cookie, "_xsrf"), None);
    }

    #[test]
    fn empty_value_is_treated_as_missing() {
        let cookie = "a=1; _xsrf=";
        assert_eq!(extract_value(cookie, "_xsrf"), None);
    }

    #[test]
    fn require_value_reports_missing_key_by_name() {
        let cookie = "a=1";
        let error = require_value(cookie, "_xsrf", "知乎").unwrap_err();
        assert!(error.contains("_xsrf"));
        assert!(error.contains("知乎"));
    }
}