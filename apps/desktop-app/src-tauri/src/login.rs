use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tauri::webview::{PageLoadEvent, PageLoadPayload};
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::platforms::shared::cookie::extract_value;

pub const EVENT_SUCCESS: &str = "login-success";
pub const EVENT_FAILED: &str = "login-failed";
const WINDOW_PREFIX: &str = "login-";

// 只允许已实现适配器的平台拉起登录窗口。未实现平台的「登录」只会把一份
// 无法用于发布的无主 Cookie 写进 credentials.json，制造「已登录」的假象。
// 与 packages/core/src/platforms/registry.ts 的 ADAPTER_IMPLEMENTED_IDS 保持一致。
const LOGIN_SUPPORTED_PLATFORMS: &[&str] = &["juejin", "csdn"];

fn is_login_supported(platform: &str) -> bool {
    LOGIN_SUPPORTED_PLATFORMS.contains(&platform)
}

pub(crate) fn ensure_login_supported(platform: &str) -> Result<(), String> {
    if is_login_supported(platform) {
        Ok(())
    } else {
        Err(format!("{} 的适配器尚未实现，暂不支持保存凭据", platform))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginVerdict {
    Completed,
    MissingKeys(Vec<String>),
    Cancelled,
}

#[derive(Default)]
struct OnceFlag(AtomicBool);

impl OnceFlag {
    fn try_acquire(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }

    fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

struct LoginSession {
    platform: String,
    finalized: OnceFlag,
    last_verdict: Mutex<Option<LoginVerdict>>,
}

impl LoginSession {
    fn new(platform: String) -> Self {
        Self {
            platform,
            finalized: OnceFlag::default(),
            last_verdict: Mutex::new(None),
        }
    }

    fn record(&self, verdict: LoginVerdict) {
        if let Ok(mut guard) = self.last_verdict.lock() {
            *guard = Some(verdict);
        }
    }

    fn last(&self) -> Option<LoginVerdict> {
        self.last_verdict.lock().ok().and_then(|guard| guard.clone())
    }
}

fn sessions() -> &'static Mutex<HashMap<String, Arc<LoginSession>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<LoginSession>>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn window_label(platform: &str) -> String {
    format!("{}{}", WINDOW_PREFIX, platform)
}

pub fn assemble_cookie_string(pairs: &[(String, String)]) -> String {
    let mut latest: HashMap<String, String> = HashMap::new();
    for (name, value) in pairs {
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.is_empty() {
            continue;
        }
        latest.insert(name.to_string(), value.to_string());
    }

    let mut names: Vec<&String> = latest.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| format!("{}={}", name, latest[name]))
        .collect::<Vec<String>>()
        .join("; ")
}

pub fn judge_cookie(cookie: &str, required_keys: &[String]) -> LoginVerdict {
    if cookie.trim().is_empty() {
        return LoginVerdict::Cancelled;
    }

    let missing: Vec<String> = required_keys
        .iter()
        .filter(|key| extract_value(cookie, key).is_none())
        .cloned()
        .collect();

    if missing.is_empty() {
        LoginVerdict::Completed
    } else {
        LoginVerdict::MissingKeys(missing)
    }
}

#[derive(Serialize, Clone)]
struct SuccessPayload {
    platform: String,
}

#[derive(Serialize, Clone)]
struct FailedPayload {
    platform: String,
    message: String,
}

fn emit_success(app: &AppHandle, platform: &str) {
    let _ = app.emit(
        EVENT_SUCCESS,
        SuccessPayload {
            platform: platform.to_string(),
        },
    );
}

fn emit_failed(app: &AppHandle, platform: &str, message: impl Into<String>) {
    let _ = app.emit(
        EVENT_FAILED,
        FailedPayload {
            platform: platform.to_string(),
            message: message.into(),
        },
    );
}

fn describe_verdict(verdict: Option<LoginVerdict>) -> String {
    match verdict {
        Some(LoginVerdict::MissingKeys(keys)) => format!(
            "登录未完成：Cookie 缺少 {}，请重新登录",
            keys.join("、")
        ),
        Some(LoginVerdict::Completed) => "登录已取消".to_string(),
        Some(LoginVerdict::Cancelled) | None => "登录已取消，未取得任何 Cookie".to_string(),
    }
}

fn finalize_login(window: WebviewWindow, session: Arc<LoginSession>, cookie: String) {
    if !session.finalized.try_acquire() {
        return;
    }
    let app = window.app_handle().clone();
    if app.get_webview_window(window.label()).is_none() {
        return;
    }
    let platform = session.platform.clone();
    match crate::persist_cookie(&app, &platform, &cookie) {
        Ok(()) => {
            emit_success(&app, &platform);
            let _ = window.close();
        }
        Err(message) => {
            emit_failed(&app, &platform, message);
        }
    }
}

fn extract_on_page_loaded(
    window: WebviewWindow,
    cookie_url: Url,
    required_keys: Vec<String>,
    session: Arc<LoginSession>,
) {
    let platform = session.platform.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cookies = match window.cookies_for_url(cookie_url) {
            Ok(cookies) => cookies,
            Err(err) => {
                if session.finalized.try_acquire() {
                    emit_failed(window.app_handle(), &platform, format!("读取 Cookie 失败: {}", err));
                }
                return;
            }
        };

        let pairs: Vec<(String, String)> = cookies
            .iter()
            .map(|cookie| (cookie.name().to_string(), cookie.value().to_string()))
            .collect();
        let cookie_string = assemble_cookie_string(&pairs);
        let verdict = judge_cookie(&cookie_string, &required_keys);

        match verdict {
            LoginVerdict::Completed => finalize_login(window, session, cookie_string),
            other => session.record(other),
        }
    });
}

#[tauri::command]
pub async fn open_login_window(
    app: AppHandle,
    platform: String,
    login_url: String,
    required_cookie_keys: Vec<String>,
) -> Result<String, String> {
    if platform.trim().is_empty() {
        return Err("平台标识不能为空".to_string());
    }

    let platform = platform.trim().to_string();
    if !is_login_supported(&platform) {
        return Err(format!(
            "{} 的适配器尚未实现，暂不支持一键登录",
            platform
        ));
    }

    let label = window_label(&platform);

    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(label);
    }

    let url = Url::parse(login_url.trim()).map_err(|e| format!("登录地址非法: {}", e))?;
    let cookie_url = url.clone();
    let session = Arc::new(LoginSession::new(platform.clone()));

    if let Ok(mut guard) = sessions().lock() {
        guard.insert(label.clone(), Arc::clone(&session));
    }

    let page_session = Arc::clone(&session);
    let window = match WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
        .title(format!("{} - 登录", platform))
        .inner_size(900.0, 720.0)
        .on_page_load(move |window, payload: PageLoadPayload<'_>| {
            if payload.event() == PageLoadEvent::Finished {
                extract_on_page_loaded(
                    window,
                    cookie_url.clone(),
                    required_cookie_keys.clone(),
                    Arc::clone(&page_session),
                );
            }
        })
        .build()
    {
        Ok(window) => window,
        Err(err) => {
            if let Ok(mut guard) = sessions().lock() {
                guard.remove(&label);
            }
            return Err(format!("创建登录窗口失败: {}", err));
        }
    };

    let close_session = Arc::clone(&session);
    let close_label = label.clone();
    let close_app = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            if let Ok(mut guard) = sessions().lock() {
                guard.remove(&close_label);
            }
            if !close_session.finalized.is_set() {
                let message = describe_verdict(close_session.last());
                emit_failed(&close_app, &close_session.platform, message);
            }
        }
    });

    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn keys(items: &[&str]) -> Vec<String> {
        items.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn login_is_rejected_for_platforms_without_adapter() {
        assert!(ensure_login_supported("juejin").is_ok());
        assert!(ensure_login_supported("csdn").is_ok());
        for id in [
            "zhihu",
            "wechat",
            "segmentfault",
            "tencent",
            "aliyun",
            "cnblogs",
        ] {
            let err = ensure_login_supported(id).expect_err("未实现适配器的平台不得保存凭据");
            assert!(err.contains("尚未实现"), "错误文案应说明适配器未实现: {}", err);
        }
    }

    #[test]
    fn assembles_all_cookies_into_single_header_string() {
        let cookie = assemble_cookie_string(&pairs(&[
            ("sessionid", "abc123"),
            ("uid", "42"),
            ("theme", "dark"),
        ]));
        assert_eq!(cookie, "sessionid=abc123; theme=dark; uid=42");
    }

    #[test]
    fn assembly_keeps_last_value_for_duplicate_name() {
        let cookie = assemble_cookie_string(&pairs(&[
            ("sessionid", "stale"),
            ("sessionid", "fresh"),
        ]));
        assert_eq!(cookie, "sessionid=fresh");
    }

    #[test]
    fn assembly_skips_empty_names_and_values() {
        let cookie = assemble_cookie_string(&pairs(&[
            ("", "orphan"),
            ("empty", ""),
            ("kept", "v"),
        ]));
        assert_eq!(cookie, "kept=v");
    }

    #[test]
    fn assembly_trims_surrounding_whitespace() {
        let cookie = assemble_cookie_string(&pairs(&[(" sessionid ", " value ")]));
        assert_eq!(cookie, "sessionid=value");
    }

    #[test]
    fn empty_cookie_is_treated_as_cancelled() {
        assert_eq!(judge_cookie("", &keys(&["sessionid"])), LoginVerdict::Cancelled);
        assert_eq!(judge_cookie("   ", &keys(&["sessionid"])), LoginVerdict::Cancelled);
    }

    #[test]
    fn cookie_with_all_required_keys_completes_login() {
        let cookie = "sessionid=abc123; uid=42";
        assert_eq!(judge_cookie(cookie, &keys(&["sessionid"])), LoginVerdict::Completed);
        assert_eq!(
            judge_cookie(cookie, &keys(&["sessionid", "uid"])),
            LoginVerdict::Completed
        );
    }

    #[test]
    fn cookie_missing_required_key_reports_missing_names() {
        assert_eq!(
            judge_cookie("uid=42", &keys(&["sessionid"])),
            LoginVerdict::MissingKeys(vec!["sessionid".to_string()])
        );
        assert_eq!(
            judge_cookie("uid=42", &keys(&["sessionid", "token"])),
            LoginVerdict::MissingKeys(vec!["sessionid".to_string(), "token".to_string()])
        );
    }

    #[test]
    fn empty_required_keys_accepts_any_non_empty_cookie() {
        assert_eq!(judge_cookie("anonymous=1", &keys(&[])), LoginVerdict::Completed);
    }

    #[test]
    fn required_key_with_blank_value_counts_as_missing() {
        assert_eq!(
            judge_cookie("sessionid=", &keys(&["sessionid"])),
            LoginVerdict::MissingKeys(vec!["sessionid".to_string()])
        );
    }

    #[test]
    fn once_flag_allows_only_first_acquisition() {
        let flag = OnceFlag::default();
        assert!(flag.try_acquire());
        assert!(!flag.try_acquire());
        assert!(flag.is_set());
    }

    #[test]
    fn session_records_latest_verdict() {
        let session = LoginSession::new("juejin".to_string());
        assert_eq!(session.last(), None);
        session.record(LoginVerdict::Cancelled);
        session.record(LoginVerdict::MissingKeys(vec!["sessionid".to_string()]));
        assert_eq!(
            session.last(),
            Some(LoginVerdict::MissingKeys(vec!["sessionid".to_string()]))
        );
    }

    #[test]
    fn missing_key_message_names_the_missing_cookie() {
        let message = describe_verdict(Some(LoginVerdict::MissingKeys(vec![
            "sessionid".to_string(),
        ])));
        assert!(message.contains("sessionid"));
    }

    #[test]
    fn cancelled_message_says_no_cookie_was_captured() {
        let message = describe_verdict(None);
        assert!(message.contains("取消"));
    }

    #[test]
    fn window_label_uses_login_prefix_for_capability_matching() {
        assert_eq!(window_label("juejin"), "login-juejin");
    }
}