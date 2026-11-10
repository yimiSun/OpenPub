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
        eprintln!(
            "[cookie] {} 命中手动配置 config_key={} bytes={}",
            platform,
            config_key,
            manual.len()
        );
        return Ok(manual);
    }

    // `credentials.json` 里的键名固定是 `{平台 id}_cookie`：`persist_cookie`
    // 写入时用的是前端传来的英文 id（如 `jianshu`），而各适配器传进来的
    // `platform` 是中文显示名（如「简书」）。用中文名拼键名会读到一个根本不
    // 存在的键，表现为「扫码登录成功、状态显示已登录，但发布时报未配置」。
    // `config_key` 在本项目里恒为 `{平台 id}_config`（前端 `save_platform_config`
    // 也是同一个约定），因此这里从它反推 id，保证读写键名严格一致。
    let platform_id = platform_id_of_config_key(config_key);
    match crate::load_stored_cookie(app, platform_id) {
        Ok(Some(stored)) if !stored.trim().is_empty() => {
            eprintln!(
                "[cookie] {} 命中加密凭据 credential_key={}_cookie bytes={}",
                platform,
                platform_id,
                stored.len()
            );
            return Ok(stored);
        }
        Ok(_) => {
            eprintln!(
                "[cookie] {} 未找到加密凭据 credential_key={}_cookie",
                platform, platform_id
            );
        }
        Err(message) => {
            eprintln!("[cookie] {} 读取加密凭据失败: {}", platform, message);
            return Err((FailureClass::NotConfigured, message));
        }
    }

    Err((
        FailureClass::NotConfigured,
        format!("{}未配置，请先登录或填写 Cookie", platform),
    ))
}

/// 从 `{平台 id}_config` 形式的配置键反推平台 id。
///
/// `persist_cookie` 用英文 id 拼 `credentials.json` 的键（`{id}_cookie`），
/// 而适配器传给 `load_from_store` 的第三个参数是中文显示名，不能用来拼键名。
/// 本项目所有平台的配置键都遵循 `{id}_config`，因此从配置键反推 id 是唯一
/// 与写入端严格对齐的可靠来源。
fn platform_id_of_config_key(config_key: &str) -> &str {
    config_key.strip_suffix("_config").unwrap_or(config_key)
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

    #[test]
    fn config_key_derives_the_same_id_used_for_credential_storage() {
        // 写入端（persist_cookie）用前端 id 拼 `{id}_cookie`；读取端必须得到同一个
        // id，否则会出现「扫码登录成功但发布报未配置」。
        assert_eq!(platform_id_of_config_key("jianshu_config"), "jianshu");
        assert_eq!(platform_id_of_config_key("juejin_config"), "juejin");
        assert_eq!(platform_id_of_config_key("51cto_config"), "51cto");
        assert_eq!(platform_id_of_config_key("tencentcloud_config"), "tencentcloud");
        assert_eq!(platform_id_of_config_key("bilibili_config"), "bilibili");
    }

    #[test]
    fn config_key_without_suffix_falls_back_to_itself() {
        assert_eq!(platform_id_of_config_key("jianshu"), "jianshu");
        assert_eq!(platform_id_of_config_key(""), "");
    }
}