use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tauri::webview::{PageLoadEvent, PageLoadPayload};
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::platforms::shared::cookie::extract_value;
use crate::platforms::webview;

pub const EVENT_SUCCESS: &str = "login-success";
pub const EVENT_FAILED: &str = "login-failed";

/// 无明确登录态 Cookie 的平台，靠页面注入脚本把「已登录」写进 `document.title`，
/// 再由 `on_document_title_changed` 读回。`LOGIN_PROBE_SCRIPT` 里的 `__MARKER__`
/// 会被替换成这个字符串的 JSON 字面量。
const LOGIN_OK_MARKER: &str = "__OPENPUB_LOGIN_OK__";

/// 登录态探针：只在「当前页不像登录页、且出现了登录后才有的 DOM 特征」时上报。
///
/// 判定分两层，缺一不可：
/// 1. URL 层面不是登录页（`loginLike()` 为 false），避免登录页自带的昵称/头像占位
///    元素造成误判；
/// 2. DOM 层面出现头像、退出链接或昵称节点。
///
/// 为什么不用 `__TAURI_INTERNALS__` 回传：远程页面里它不可靠（见 webview 模块
/// 顶部注释）。`document.title` 是页面自己的属性，Rust 侧通过原生窗口标题事件
/// 就能读到，是这条链路唯一稳定的纯 DOM 回传方式。
const LOGIN_PROBE_SCRIPT: &str = r#"(function () {
  if (window.__OPENPUB_LOGIN_PROBE__) { return; }
  window.__OPENPUB_LOGIN_PROBE__ = true;
  var MARKER = __MARKER__;
  var HINTS = ['login', 'sign_in', 'signin', 'sign-in', 'passport', 'sso'];
  function loginLike() {
    try {
      var host = (location.hostname || '').toLowerCase();
      if (host.indexOf('passport') >= 0 || host.indexOf('login') >= 0) { return true; }
      var path = ((location.pathname || '') + (location.search || '')).toLowerCase();
      for (var i = 0; i < HINTS.length; i++) {
        if (path.indexOf(HINTS[i]) >= 0) { return true; }
      }
    } catch (e) { return true; }
    return false;
  }
  function loggedIn() {
    var selectors = [
      'img.avatar', '.avatar img', '.user-avatar img', '.header-avatar img',
      '[class*="avatar"] img', 'a[href*="logout"]', 'a[href*="signout"]',
      'a[href*="sign_out"]', '[class*="user-name"]', '[class*="nickname"]'
    ];
    for (var i = 0; i < selectors.length; i++) {
      try { if (document.querySelector(selectors[i])) { return true; } } catch (e) {}
    }
    return false;
  }
  var reported = false;
  function tick() {
    try {
      if (reported) { document.title = MARKER; return; }
      if (!loginLike() && loggedIn()) { reported = true; document.title = MARKER; }
    } catch (e) {}
  }
  setInterval(tick, 1000);
  tick();
})();"#;

/// 拼装探针脚本：标记值一律走 JSON 字面量，避免引号破坏脚本结构。
fn login_probe_script() -> String {
    let marker = serde_json::to_string(LOGIN_OK_MARKER)
        .unwrap_or_else(|_| "\"\"".to_string());
    LOGIN_PROBE_SCRIPT.replace("__MARKER__", &marker)
}

const WINDOW_PREFIX: &str = "login-";

// 只允许已实现适配器的平台拉起登录窗口。未实现平台的「登录」只会把一份
// 无法用于发布的无主 Cookie 写进 credentials.json，制造「已登录」的假象。
// 与 packages/core/src/platforms/registry.ts 的 ADAPTER_IMPLEMENTED_IDS 保持一致。
//
// 知乎不在此列：它的登录与发布共用 `channel-zhihu` 通道窗口（挂载了
// `.on_new_window` 拦截，微信 / QQ 授权页才能正常弹出），入口是
// `open_channel_login_window`，不走这里的 `login-{platform}` 窗口。
//
// `adapterChannel: "webview"` 的平台同样不在此列（见 project_rules.md）：它们的
// 登录态由通道窗口自己持有，不写 DPAPI Cookie。
//
// 简书自 v1.0.0 起也移出本列：它的接口按客户端 TLS/HTTP 指纹做风控，Rust 侧
// 无论怎么补头都是 `403 code 9 异常请求`，发布已改到 `channel-jianshu` 页面
// 上下文里执行，登录随之复用同一通道窗口（入口 `open_channel_login_window`）。
const LOGIN_SUPPORTED_PLATFORMS: &[&str] = &[
    "juejin",
    "csdn",
    "51cto",
    "oschina",
    "tencentcloud",
    "aliyun",
    "baijiahao",
    "netease",
    "bilibili",
    "weibo",
    "toutiao",
];

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
    /// 有明确登录态 Cookie 的平台直接按 key 判定；为空表示该平台没有可靠的
    /// 登录标志 Cookie（简书、51CTO 等），必须靠 URL 跳转或 DOM 特征确认。
    required_keys: Vec<String>,
    /// 是否已见过登录页。用于「从 sign_in 跳回首页」这条 URL 判据：
    /// 只有先见过登录页，跳离登录页才等价于登录成功。
    seen_login_page: AtomicBool,
    finalized: OnceFlag,
    last_verdict: Mutex<Option<LoginVerdict>>,
}

impl LoginSession {
    fn new(platform: String, required_keys: Vec<String>) -> Self {
        Self {
            platform,
            required_keys,
            seen_login_page: AtomicBool::new(false),
            finalized: OnceFlag::default(),
            last_verdict: Mutex::new(None),
        }
    }

    /// 没有明确 Cookie 键的平台不能「一开窗就判定已登录」，必须等确认信号。
    fn requires_confirmation(&self) -> bool {
        self.required_keys.is_empty()
    }

    fn mark_seen_login_page(&self) {
        self.seen_login_page.store(true, Ordering::SeqCst);
    }

    fn has_seen_login_page(&self) -> bool {
        self.seen_login_page.load(Ordering::SeqCst)
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

/// 供通道登录（如知乎的 `channel-zhihu` 窗口）复用同一套前端事件。
pub(crate) fn emit_success(app: &AppHandle, platform: &str) {
    let _ = app.emit(
        EVENT_SUCCESS,
        SuccessPayload {
            platform: platform.to_string(),
        },
    );
}

/// 供通道登录复用：窗口被关闭、超时等失败场景都要通知前端解除「窗口已打开」状态。
pub(crate) fn emit_failed(app: &AppHandle, platform: &str, message: impl Into<String>) {
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

/// 判断一个地址是否「像登录页」。
///
/// 只做保守的正向匹配：host 含 `passport` / `login`，或 path + query 命中登录相关
/// 关键词。命中意味着「这里还没有登录态」，绝不能据此抓 Cookie 收尾。
pub fn looks_like_login_page(url: &Url) -> bool {
    const HINTS: &[&str] = &["login", "sign_in", "signin", "sign-in", "passport", "sso"];
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if host.contains("passport") || host.contains("login") {
        return true;
    }
    let path = format!("{}{}", url.path(), url.query().unwrap_or("")).to_ascii_lowercase();
    HINTS.iter().any(|hint| path.contains(hint))
}

/// 登录入口与业务站点不在同一域时，需要额外补抓的地址。
///
/// 阿里云的统一登录在 `account.aliyun.com`，而发文章用的是
/// `developer.aliyun.com`。`c_csrf` 这个防 CSRF 令牌是**业务子域下发**的
/// host-only Cookie，只在登录页那一侧抓的话根本取不到，表现为「扫码登录成功、
/// 状态显示已登录，但发布时提示 Cookie 中缺少 c_csrf」。
///
/// WebView2 的 `cookies_for_url` 不接受通配域名，只能按具体 URL 查询：传入
/// `developer.aliyun.com` 会同时返回 `.aliyun.com` 的域级 Cookie 与该子域的
/// host-only Cookie，因此这里列业务域即可覆盖「放宽到 .aliyun.com」的诉求。
fn supplemental_cookie_urls(platform: &str) -> Vec<Url> {
    let raw: &[&str] = match platform {
        "aliyun" => &["https://developer.aliyun.com/"],
        _ => &[],
    };
    raw.iter()
        .filter_map(|url| Url::parse(url).ok())
        .collect()
}

/// 把业务域的补充地址并进待抓取列表，去重后返回。
fn with_supplemental_urls(platform: &str, urls: Vec<Url>) -> Vec<Url> {
    let mut merged = urls;
    for extra in supplemental_cookie_urls(platform) {
        if !merged.iter().any(|url| url.as_str() == extra.as_str()) {
            merged.push(extra);
        }
    }
    merged
}

/// 把多个地址下的 Cookie 合并成一条请求头字符串。
///
/// 跨域登录（`passport.*` 授权后跳回主站）只有把登录页与当前页都查一遍，
/// 才能同时拿到主站与授权域的 Cookie。
///
/// 走的是 WebView2 的 Cookie 管理器（`cookies_for_url`），它按 URL 返回该域
/// 全部 Cookie，**包含 `HttpOnly`**。简书等平台的会话 Cookie 只在 `HttpOnly`
/// 里，若这里拿不到就必须换抓取方式，所以把键名打出来便于核对（不打印值）。
fn collect_cookie_string(window: &WebviewWindow, urls: &[Url]) -> Result<String, String> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for url in urls {
        let cookies = window
            .cookies_for_url(url.clone())
            .map_err(|err| format!("读取 Cookie 失败: {}", err))?;
        pairs.extend(
            cookies
                .iter()
                .map(|cookie| (cookie.name().to_string(), cookie.value().to_string())),
        );
    }
    let mut names: Vec<&str> = pairs.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    eprintln!(
        "[login] 抓取 Cookie 共 {} 项 keys=[{}]",
        pairs.len(),
        names.join(",")
    );
    Ok(assemble_cookie_string(&pairs))
}

fn extract_by_keys(
    window: WebviewWindow,
    cookie_url: Url,
    required_keys: Vec<String>,
    session: Arc<LoginSession>,
) {
    let platform = session.platform.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let urls = with_supplemental_urls(&platform, vec![cookie_url]);
        let cookie_string = match collect_cookie_string(&window, &urls) {
            Ok(cookie) => cookie,
            Err(message) => {
                if session.finalized.try_acquire() {
                    emit_failed(window.app_handle(), &platform, message);
                }
                return;
            }
        };
        let verdict = judge_cookie(&cookie_string, &required_keys);

        match verdict {
            LoginVerdict::Completed => finalize_login(window, session, cookie_string),
            other => session.record(other),
        }
    });
}

/// 确认登录态后抓取全量 Cookie 并收尾。
fn capture_and_finalize(window: WebviewWindow, urls: Vec<Url>, session: Arc<LoginSession>) {
    if session.finalized.is_set() {
        return;
    }
    let platform = session.platform.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let urls = with_supplemental_urls(&platform, urls);
        let cookie_string = match collect_cookie_string(&window, &urls) {
            Ok(cookie) => cookie,
            Err(message) => {
                if session.finalized.try_acquire() {
                    emit_failed(window.app_handle(), &platform, message);
                }
                return;
            }
        };
        if cookie_string.trim().is_empty() {
            session.record(LoginVerdict::Cancelled);
            return;
        }
        finalize_login(window, session, cookie_string);
    });
}

/// 页面加载完成后的判定入口，两条路径泾渭分明：
///
/// - 有明确登录标志 Cookie 的平台：照旧按 key 判定，缺 key 只记录不关窗；
/// - 没有明确登录标志的平台（简书、51CTO 等）：绝不在一开窗就抓 Cookie 收尾，
///   而是等「URL 从登录页跳走」或「DOM 出现头像/昵称」这两个确认信号。
fn handle_page_load(
    window: WebviewWindow,
    page_url: Url,
    cookie_url: Url,
    session: Arc<LoginSession>,
) {
    if session.finalized.is_set() {
        return;
    }

    if !session.requires_confirmation() {
        let required_keys = session.required_keys.clone();
        extract_by_keys(window, cookie_url, required_keys, session);
        return;
    }

    if looks_like_login_page(&page_url) {
        session.mark_seen_login_page();
        let _ = window.eval(&login_probe_script());
        return;
    }

    if session.has_seen_login_page() {
        // 从 sign_in 跳回首页：URL 判据已确认登录成功，直接抓 Cookie 收尾。
        capture_and_finalize(window, vec![cookie_url, page_url], session);
        return;
    }

    // 首次落地就不是登录页（如网易号后台直接开在首页）：登录与否都可能停在这里，
    // 因此交给 DOM 探针确认，不能在此直接抓 Cookie。
    let _ = window.eval(&login_probe_script());
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
    let session = Arc::new(LoginSession::new(platform.clone(), required_cookie_keys));

    if let Ok(mut guard) = sessions().lock() {
        guard.insert(label.clone(), Arc::clone(&session));
    }

    let page_session = Arc::clone(&session);
    let page_cookie_url = cookie_url.clone();
    let title_session = Arc::clone(&session);
    let title_cookie_url = cookie_url.clone();
    let nav_app = app.clone();
    let nav_label = label.clone();
    let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
        .title(format!("{} - 登录", platform))
        .inner_size(900.0, 720.0);
    // 微信 / QQ 登录按钮走 `window.open`。登录窗口必须与通道窗口共用同一份拦截，
    // 否则点击授权按钮会完全无反应（详见 webview 模块的实现注释）。
    let window = match webview::with_same_window_navigation(builder, nav_app, nav_label)
        // 简书的「微信 / QQ 登录」不是 window.open，而是带 `target="_blank"` 的
        // 普通 `<a>`。只挂 `.on_new_window` 时 WebView2 先接管了这次点击，事后
        // 补发的跳转经常来不及生效，表现为点击完全没反应。初始化脚本在文档
        // 开始解析前就改写这类链接，点击直接在当前窗口内导航。
        .initialization_script(webview::SAME_WINDOW_LINK_SCRIPT)
        // 兜底放行：微信 / QQ 授权页在第三方域名（`open.weixin.qq.com`、
        // `graph.qq.com` 等），必须允许顶层导航过去，否则授权会在跳转瞬间
        // 被静默拦截。这里只拒绝 `javascript:` / `file:` 这类非 web scheme。
        .on_navigation(|target| matches!(target.scheme(), "http" | "https" | "about"))
        .on_page_load(move |window, payload: PageLoadPayload<'_>| {
            if payload.event() == PageLoadEvent::Finished {
                handle_page_load(
                    window,
                    payload.url().clone(),
                    page_cookie_url.clone(),
                    Arc::clone(&page_session),
                );
            }
        })
        // DOM 探针把「已登录」写进 `document.title`，这里读回标记后立即抓 Cookie。
        // 覆盖 SPA 站点的无跳转登录：这类页面不会产生新的 `on_page_load`，
        // 只靠 URL 判据永远等不到确认信号。
        .on_document_title_changed(move |window, title| {
            if title != LOGIN_OK_MARKER {
                return;
            }
            let mut urls = vec![title_cookie_url.clone()];
            if let Ok(current) = window.url() {
                if current.as_str() != title_cookie_url.as_str() {
                    urls.push(current);
                }
            }
            capture_and_finalize(window, urls, Arc::clone(&title_session));
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
        for id in ["juejin", "csdn", "51cto", "oschina", "tencentcloud", "aliyun",
                   "baijiahao", "netease", "bilibili", "weibo", "toutiao"] {
            assert!(ensure_login_supported(id).is_ok(), "{} 应支持一键登录", id);
        }
        // 知乎、简书的登录走各自的通道窗口，不走 `login-{platform}`，因此这里必须拒绝。
        for id in ["zhihu", "jianshu"] {
            assert!(
                ensure_login_supported(id).is_err(),
                "{} 的登录应由通道窗口接管",
                id
            );
        }
        for id in ["zhihu", "jianshu", "wechat", "segmentfault", "xiaohongshu", "douyin", "cnblogs"] {
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
        let session = LoginSession::new("juejin".to_string(), keys(&["sessionid"]));
        assert_eq!(session.last(), None);
        session.record(LoginVerdict::Cancelled);
        session.record(LoginVerdict::MissingKeys(vec!["sessionid".to_string()]));
        assert_eq!(
            session.last(),
            Some(LoginVerdict::MissingKeys(vec!["sessionid".to_string()]))
        );
    }

    #[test]
    fn session_without_required_keys_demands_explicit_confirmation() {
        let strict = LoginSession::new("juejin".to_string(), keys(&["sessionid"]));
        assert!(!strict.requires_confirmation());

        // 51CTO 这类没有登录标志 Cookie 的平台，必须等确认信号。
        let loose = LoginSession::new("51cto".to_string(), Vec::new());
        assert!(loose.requires_confirmation());
        assert!(!loose.has_seen_login_page());
        loose.mark_seen_login_page();
        assert!(loose.has_seen_login_page());
    }

    #[test]
    fn login_page_detection_matches_passport_and_path_hints() {
        for raw in [
            "https://passport.bilibili.com/login",
            "https://www.jianshu.com/sign_in",
            "https://blog.51cto.com/login",
            "https://www.oschina.net/home/login",
            "https://cloud.tencent.com/login",
            "https://account.aliyun.com/login/login.htm",
            "https://weibo.com/login.php?url=home",
        ] {
            assert!(looks_like_login_page(&Url::parse(raw).unwrap()), "应识别为登录页: {}", raw);
        }

        for raw in [
            "https://www.jianshu.com/",
            "https://mp.163.com/",
            "https://mp.toutiao.com/",
            "https://baijiahao.baidu.com/",
            "https://juejin.cn/",
        ] {
            assert!(
                !looks_like_login_page(&Url::parse(raw).unwrap()),
                "不应把内容页当成登录页: {}",
                raw
            );
        }
    }

    #[test]
    fn probe_script_embeds_marker_as_json_literal() {
        let script = login_probe_script();
        assert!(script.contains(&format!("\"{}\"", LOGIN_OK_MARKER)));
        assert!(!script.contains("__MARKER__"), "占位符必须被替换");
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