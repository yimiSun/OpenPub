use std::cmp::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const DEFAULT_LATEST_URL: &str =
    "https://gitee.com/tianyeHome/OpenPub/raw/master/latest.json";

const FETCH_TIMEOUT_SECS: u64 = 10;

const USER_AGENT: &str = "OpenPub/updater";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateState {
    UpToDate,
    UpdateAvailable,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub state: UpdateState,
    pub current: String,
    pub latest: Option<String>,
    pub has_update: bool,
    pub download_url: Option<String>,
    pub notes: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct LatestInfo {
    version: String,
    #[serde(default, alias = "download_url")]
    download_url: Option<String>,
    #[serde(default)]
    notes: Option<String>,
}

pub fn parse_version(raw: &str) -> Result<Vec<u64>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("版本号为空".to_string());
    }
    let core = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    if core.is_empty() {
        return Err(format!("版本号格式不合法: {}", raw));
    }

    let mut segments = Vec::new();
    for segment in core.split('.') {
        if segment.is_empty() {
            return Err(format!("版本号含空段: {}", raw));
        }
        if !segment.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("版本号含非数字段「{}」: {}", segment, raw));
        }
        let value: u64 = segment
            .parse()
            .map_err(|_| format!("版本号段超出范围「{}」: {}", segment, raw))?;
        segments.push(value);
    }
    Ok(segments)
}

pub fn compare_versions(current: &str, latest: &str) -> Result<Ordering, String> {
    let mut current_segments = parse_version(current)?;
    let mut latest_segments = parse_version(latest)?;
    let width = current_segments.len().max(latest_segments.len());
    current_segments.resize(width, 0);
    latest_segments.resize(width, 0);
    Ok(current_segments.cmp(&latest_segments))
}

fn failed(current: &str, message: impl Into<String>) -> UpdateCheck {
    UpdateCheck {
        state: UpdateState::Failed,
        current: current.to_string(),
        latest: None,
        has_update: false,
        download_url: None,
        notes: None,
        message: message.into(),
    }
}

pub fn evaluate(fetched: Result<String, String>, current: &str) -> UpdateCheck {
    let body = match fetched {
        Ok(body) => body,
        Err(message) => return failed(current, message),
    };

    let info: LatestInfo = match serde_json::from_str(&body) {
        Ok(info) => info,
        Err(err) => {
            return failed(current, format!("远端版本文件格式错误: {}", err));
        }
    };

    if info.version.trim().is_empty() {
        return failed(current, "远端版本文件缺少 version 字段");
    }

    let ordering = match compare_versions(current, &info.version) {
        Ok(ordering) => ordering,
        Err(err) => return failed(current, format!("版本号无法比较: {}", err)),
    };

    let has_update = ordering == Ordering::Less;

    if has_update && info.download_url.as_deref().unwrap_or("").trim().is_empty() {
        return failed(current, "远端版本文件缺少 downloadUrl 字段");
    }

    let message = if has_update {
        format!("发现新版本 {}", info.version.trim())
    } else {
        "当前已是最新版本".to_string()
    };

    UpdateCheck {
        state: if has_update {
            UpdateState::UpdateAvailable
        } else {
            UpdateState::UpToDate
        },
        current: current.to_string(),
        latest: Some(info.version.trim().to_string()),
        has_update,
        download_url: info
            .download_url
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty()),
        notes: info
            .notes
            .map(|notes| notes.trim().to_string())
            .filter(|notes| !notes.is_empty()),
        message,
    }
}

async fn fetch_latest(url: &str) -> Result<String, String> {
    if url.trim().is_empty() {
        return Err("未配置更新源".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|err| format!("创建 HTTP 客户端失败: {}", err))?;

    let response = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|err| format!("网络请求失败: {}", err))?;

    if !response.status().is_success() {
        return Err(format!("网络请求失败: HTTP {}", response.status().as_u16()));
    }

    response
        .text()
        .await
        .map_err(|err| format!("读取远端版本文件失败: {}", err))
}

#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> UpdateCheck {
    let current = app.package_info().version.to_string();
    let fetched = fetch_latest(DEFAULT_LATEST_URL).await;
    evaluate(fetched, &current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latest_json(version: &str, download_url: &str, notes: &str) -> String {
        serde_json::json!({
            "version": version,
            "downloadUrl": download_url,
            "notes": notes,
        })
        .to_string()
    }

    #[test]
    fn parse_version_accepts_plain_and_prefixed_numbers() {
        assert_eq!(parse_version("1.2.3").unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_version("v1.2.3").unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_version(" 1.2.3 ").unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_version("1").unwrap(), vec![1]);
    }

    #[test]
    fn parse_version_rejects_empty_and_non_numeric() {
        assert!(parse_version("").is_err());
        assert!(parse_version("   ").is_err());
        assert!(parse_version("v").is_err());
        assert!(parse_version("1..3").is_err());
        assert!(parse_version("1.0.x").is_err());
        assert!(parse_version("1.0.3-beta").is_err());
        assert!(parse_version("1.0.3 ").is_ok());
    }

    #[test]
    fn compare_equal_versions_is_equal() {
        assert_eq!(compare_versions("1.0.0", "1.0.0").unwrap(), Ordering::Equal);
        assert_eq!(compare_versions("0.1.0", "0.1.0").unwrap(), Ordering::Equal);
        assert_eq!(compare_versions("v1.2.3", "1.2.3").unwrap(), Ordering::Equal);
    }

    #[test]
    fn compare_pads_missing_segments_with_zero() {
        assert_eq!(compare_versions("1.0", "1.0.0").unwrap(), Ordering::Equal);
        assert_eq!(compare_versions("1", "1.0.0").unwrap(), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0", "1").unwrap(), Ordering::Equal);
        assert_eq!(compare_versions("1.0.1", "1.0").unwrap(), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1.0.1").unwrap(), Ordering::Less);
    }

    #[test]
    fn compare_numeric_segments_not_lexicographic() {
        assert_eq!(
            compare_versions("0.1.9", "0.1.10").unwrap(),
            Ordering::Less
        );
        assert_eq!(
            compare_versions("0.1.10", "0.1.9").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.9.0", "1.10.0").unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn compare_fails_when_either_side_has_non_numeric_segment() {
        assert!(compare_versions("1.0.0", "1.0.x").is_err());
        assert!(compare_versions("abc", "1.0.0").is_err());
    }

    #[test]
    fn evaluate_reports_up_to_date_when_versions_equal() {
        let result = evaluate(
            Ok(latest_json("0.1.0", "https://example.com/dl", "首个版本")),
            "0.1.0",
        );
        assert_eq!(result.state, UpdateState::UpToDate);
        assert!(!result.has_update);
        assert_eq!(result.latest.as_deref(), Some("0.1.0"));
        assert_eq!(result.message, "当前已是最新版本");
    }

    #[test]
    fn evaluate_reports_update_when_remote_is_newer() {
        let result = evaluate(
            Ok(latest_json(
                "0.1.10",
                "https://example.com/dl",
                "修复若干问题",
            )),
            "0.1.9",
        );
        assert_eq!(result.state, UpdateState::UpdateAvailable);
        assert!(result.has_update);
        assert_eq!(result.latest.as_deref(), Some("0.1.10"));
        assert_eq!(result.download_url.as_deref(), Some("https://example.com/dl"));
        assert_eq!(result.notes.as_deref(), Some("修复若干问题"));
        assert_eq!(result.message, "发现新版本 0.1.10");
    }

    #[test]
    fn evaluate_reports_up_to_date_when_remote_is_older() {
        let result = evaluate(
            Ok(latest_json("0.1.9", "https://example.com/dl", "")),
            "0.2.0",
        );
        assert_eq!(result.state, UpdateState::UpToDate);
        assert!(!result.has_update);
        assert!(result.download_url.is_some());
    }

    #[test]
    fn evaluate_pads_missing_segment_in_remote_version() {
        let result = evaluate(Ok(latest_json("1.0", "https://example.com/dl", "")), "1.0.0");
        assert_eq!(result.state, UpdateState::UpToDate);
        assert!(!result.has_update);
        assert_eq!(result.latest.as_deref(), Some("1.0"));
    }

    #[test]
    fn evaluate_fails_when_remote_version_has_non_numeric_segment() {
        let result = evaluate(
            Ok(latest_json("1.0.x", "https://example.com/dl", "")),
            "1.0.0",
        );
        assert_eq!(result.state, UpdateState::Failed);
        assert!(!result.has_update);
        assert_eq!(result.latest, None);
        assert!(result.message.contains("版本号无法比较"), "{}", result.message);
    }

    #[test]
    fn evaluate_fails_when_network_request_fails() {
        let result = evaluate(
            Err("网络请求失败: error sending request".to_string()),
            "0.1.0",
        );
        assert_eq!(result.state, UpdateState::Failed);
        assert!(!result.has_update);
        assert_eq!(result.current, "0.1.0");
        assert!(result.message.contains("网络请求失败"), "{}", result.message);
    }

    #[test]
    fn evaluate_fails_when_update_source_is_missing() {
        let result = evaluate(Err("未配置更新源".to_string()), "0.1.0");
        assert_eq!(result.state, UpdateState::Failed);
        assert_eq!(result.message, "未配置更新源");
    }

    #[test]
    fn evaluate_fails_when_body_is_not_json() {
        let result = evaluate(Ok("<html>404</html>".to_string()), "0.1.0");
        assert_eq!(result.state, UpdateState::Failed);
        assert!(result.message.contains("远端版本文件格式错误"));
    }

    #[test]
    fn evaluate_fails_when_version_field_is_missing() {
        let body = serde_json::json!({ "downloadUrl": "https://example.com/dl" }).to_string();
        let result = evaluate(Ok(body), "0.1.0");
        assert_eq!(result.state, UpdateState::Failed);
        assert!(result.message.contains("远端版本文件格式错误"));
    }

    #[test]
    fn evaluate_fails_when_download_url_missing_but_update_available() {
        let body = serde_json::json!({ "version": "0.2.0" }).to_string();
        let result = evaluate(Ok(body), "0.1.0");
        assert_eq!(result.state, UpdateState::Failed);
        assert!(result.message.contains("downloadUrl"));
    }

    #[test]
    fn evaluate_tolerates_missing_download_url_when_up_to_date() {
        let body = serde_json::json!({ "version": "0.1.0" }).to_string();
        let result = evaluate(Ok(body), "0.1.0");
        assert_eq!(result.state, UpdateState::UpToDate);
        assert_eq!(result.download_url, None);
        assert_eq!(result.notes, None);
    }

    #[test]
    fn evaluate_accepts_snake_case_download_url_alias() {
        let body = serde_json::json!({
            "version": "0.2.0",
            "download_url": "https://example.com/dl",
        })
        .to_string();
        let result = evaluate(Ok(body), "0.1.0");
        assert_eq!(result.state, UpdateState::UpdateAvailable);
        assert_eq!(result.download_url.as_deref(), Some("https://example.com/dl"));
    }

    #[test]
    fn update_check_serializes_to_camel_case() {
        let result = evaluate(
            Ok(latest_json("0.2.0", "https://example.com/dl", "说明")),
            "0.1.0",
        );
        let json = serde_json::to_value(&result).unwrap();
        let object = json.as_object().expect("UpdateCheck 应序列化为对象");
        assert!(object.contains_key("hasUpdate"));
        assert!(object.contains_key("downloadUrl"));
        assert!(!object.contains_key("download_url"));
        assert!(!object.contains_key("has_update"));
        assert_eq!(object["state"], serde_json::json!("updateAvailable"));
    }
}