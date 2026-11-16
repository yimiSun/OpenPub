  use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

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
/// 探针的中间态前缀：`__OPENPUB_PROBE__|status|...`。只用于日志诊断，绝不触发抓取。
const PROBE_PENDING_PREFIX: &str = "__OPENPUB_PROBE__";
/// 探针诊断前缀：页面 DOM 特征为真、但 `document.cookie` 里查不到任何鉴权字段。
/// Rust 侧据此打日志说明「探针认出来了但 cookie 不可信」，既不能抓也不能算失败，
/// 继续等待（用户可能只是还没走完登录）。
const PROBE_NO_AUTH_PREFIX: &str = "__OPENPUB_PROBE_NOAUTH__";

/// 登录态探针：**唯一**的抓 Cookie 触发源。
///
/// 判定分三层，全部满足才上报 `LOGIN_OK_MARKER`：
/// 1. URL 层面不是登录页（`loginLike()` 为 false），避免登录页自带的昵称/头像
///    占位元素造成误判。**例外**：51CTO 把已登录与未登录状态都渲染在
///    `blog.51cto.com/login`，已登录用户点「配置」不会跳转，此时 URL 判据必然
///    失败。因此该路径下改为只看 DOM：出现「退出」这类只有登录后才有、且匿名
///    页面绝不存在的入口，即可越过第 1 层。
/// 2. DOM 层面出现登录后才有的特征（头像、退出链接、昵称、51CTO 的创作中心/我的
///    博客等文本链接）。仅此一处不够：退出登录后的页面仍可能残留头像节点。
/// 3. `document.cookie` 里必须存在真正的鉴权会话字段。统计类 Cookie（`Hm_*`、
///    `_ga`、`BIDUPSID`、`login_from` 等）无论多少都不算登录。
///
/// 为什么不用 `__TAURI_INTERNALS__` 回传：远程页面里它不可靠（见 webview 模块
/// 顶部注释）。`document.title` 是页面自己的属性，Rust 侧通过原生窗口标题事件
/// 就能读到，是这条链路唯一稳定的纯 DOM 回传方式。
///
/// 探针 title 协议：
/// - `__OPENPUB_PROBE__|...`：tick 中间态，仅诊断
/// - `__OPENPUB_PROBE_NOAUTH__|...`：DOM 命中但没有鉴权 Cookie，仅诊断
/// - `__OPENPUB_LOGIN_OK__`：三层全部通过，Rust 侧据此抓取并收尾
const LOGIN_PROBE_SCRIPT: &str = r#"(function () {
  if (window.__OPENPUB_LOGIN_PROBE__) { return; }
  window.__OPENPUB_LOGIN_PROBE__ = true;
  var MARKER = __MARKER__;
  var PENDING = __PENDING__;
  var NOAUTH = __NOAUTH__;
  var HINTS = ['login', 'sign_in', 'signin', 'sign-in', 'passport', 'sso'];
  function loginLike() {
    try {
      var host = (location.hostname || '').toLowerCase();
      if (host.indexOf('passport') >= 0 || host.indexOf('login') >= 0) { return true; }
      // 只看 pathname，不看 search：登录成功后平台会把目标地址塞进查询参数
      // （51CTO 的 `?reback=...login-success...`），把 search 算进来会让这个
      // 「已经登录成功」的落地页被当成登录页，探针永远不上报。
      var path = (location.pathname || '').toLowerCase();
      if (path.indexOf('login-success') >= 0 || path.indexOf('login_success') >= 0) {
        return false;
      }
      for (var i = 0; i < HINTS.length; i++) {
        if (path.indexOf(HINTS[i]) >= 0) { return true; }
      }
    } catch (e) { return true; }
    return false;
  }
  function countMatches() {
    var selectors = [
      'img.avatar', '.avatar img', '.user-avatar img', '.header-avatar img',
      '[class*="avatar"] img', 'a[href*="logout"]', 'a[href*="signout"]',
      'a[href*="sign_out"]', '[class*="user-name"]', '[class*="nickname"]',
      // 个人主页 / 后台入口：退出登录后这些节点会消失
      'a[href*="/u_"]', 'a[href*="/blogger"]', 'a[href*="create"]',
      '[class*="user-box"]', '[class*="user-info"]', '[data-user-id]',
      '[class*="blogger-menu"]', '[class*="create-center"]'
    ];
    var hits = 0;
    for (var i = 0; i < selectors.length; i++) {
      try { if (document.querySelector(selectors[i])) { hits++; } } catch (e) {}
    }
    return hits;
  }
  // 文本特征：51CTO 等站点的登录后菜单是纯文字链接，没有稳定的 class/href。
  // 退出登录后这些入口会一并消失，所以同样能作为登录标志。
  var TEXT_HINTS = ['退出', '我的博客', '创作中心', '写文章', '个人中心', '发文章'];
  function textMatches() {
    var found = 0;
    try {
      var links = document.querySelectorAll('a, li, span, div');
      for (var i = 0; i < links.length && i < 3000; i++) {
        var txt = (links[i].textContent || '').trim();
        if (!txt || txt.length > 12) { continue; }
        for (var j = 0; j < TEXT_HINTS.length; j++) {
          if (txt.indexOf(TEXT_HINTS[j]) >= 0) { found++; break; }
        }
      }
    } catch (e) {}
    return found;
  }
  // 鉴权字段：只有这些才算「真的登录了」。统计 Cookie 一律不算。
  // 刻意不收 'sid' / 'uid' / 'login' 这类短词或弱词：BIDUPSID 含 sid、
  // login_from 含 login，写成子串匹配会把匿名 Cookie 放进来。
  // 'PHPSESSID' 同样不能只看名字：51CTO 匿名访客也会被下发一个匿名 PHPSESSID，
  // 真机实测因此存下过一份「看似已登录、实际未登录」的凭据。51CTO 改用
  // 专属字段值判定（见 fiftyOneCtoLoggedIn）。
  var AUTH_KEYS = ['token', 'session', 'userid', 'user_id', 'access_token',
                   'auth', 'ssotk', 'sid_token'];
  function cookieMap() {
    var map = {};
    try {
      var raw = document.cookie || '';
      var parts = raw.split(';');
      for (var i = 0; i < parts.length; i++) {
        var eq = parts[i].indexOf('=');
        if (eq <= 0) { continue; }
        map[parts[i].slice(0, eq).trim().toLowerCase()] = parts[i].slice(eq + 1).trim();
      }
    } catch (e) {}
    return map;
  }
  function authCookie() {
    var map = cookieMap();
    for (var name in map) {
      if (!map[name]) { continue; }
      if (name === 'phpsessid') { continue; }
      for (var j = 0; j < AUTH_KEYS.length; j++) {
        if (name.indexOf(AUTH_KEYS[j]) >= 0) { return name; }
      }
    }
    return 'none';
  }
  // 51CTO 的匿名访客也会拿到 pub_auth_profile / pub_cookietime，只是值为空或 0。
  // 只有值被填过才说明真的登录了。
  function fiftyOneCtoLoggedIn() {
    var map = cookieMap();
    var ok = function (v) { return !!v && v !== '0' && v !== 'null'; };
    return ok(map['pub_auth_profile']) || ok(map['pub_cookietime']);
  }
  var reported = false;
  var lastTitle = '';
  function setTitleIfChanged(t) {
    if (t !== lastTitle) { lastTitle = t; document.title = t; }
  }
  function tick() {
    try {
      if (reported) { setTitleIfChanged(MARKER); return; }
      var login = loginLike();
      var hits = countMatches();
      var texts = textMatches();
      var auth = authCookie();
      var cto = fiftyOneCtoLoggedIn();
      // 登录页一律不上报，这是硬性一票否决。
      //
      // 曾经的写法允许「登录页上出现退出/创作中心文字」就越过 URL 判据，结果被
      // 真机证伪：`blog.51cto.com/login` 自带整站导航栏，「创作中心」「我的博客」
      // 这些文字在**匿名**状态下同样存在，于是探针在用户扫码前就上报命中，窗口
      // 两秒后自动关闭并保存了一份匿名 Cookie。文字特征不足以区分登录与否，
      // 唯一可靠的信号是「真的离开了登录页」。
      var domOk = !login && (hits > 0 || texts > 0);
      var authOk = auth !== 'none' || cto;
      if (domOk && authOk) {
        reported = true;
        setTitleIfChanged(MARKER);
        return;
      }
      var diag = 'loginLike=' + login +
                 '|hits=' + hits + '|texts=' + texts + '|cto=' + cto +
                 '|auth=' + auth + '|path=' + (location.pathname || '');
      // DOM 已经认出来了却拿不到鉴权 Cookie：单独一个前缀，便于日志区分
      setTitleIfChanged((domOk ? NOAUTH : PENDING) + '|' + diag);
    } catch (e) {
      setTitleIfChanged(PENDING + '|error|' + String(e).slice(0, 80));
    }
  }
  setInterval(tick, 1000);
  tick();
})();"#;

/// 拼装探针脚本：标记值一律走 JSON 字面量，避免引号破坏脚本结构。
fn login_probe_script() -> String {
    let literal = |value: &str| {
        serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
    };
    LOGIN_PROBE_SCRIPT
        .replace("__MARKER__", &literal(LOGIN_OK_MARKER))
        .replace("__PENDING__", &literal(PROBE_PENDING_PREFIX))
        .replace("__NOAUTH__", &literal(PROBE_NO_AUTH_PREFIX))
}

const WINDOW_PREFIX: &str = "login-";

/// 确认登录态之后、读取 Cookie 之前的静置时间。
///
/// 「页面已经跳走」不等于「Cookie 已经落盘」：WebView2 处理 302 响应里的
/// `Set-Cookie` 与本进程调用 `cookies_for_url` 之间存在毫秒级时间差。实测扫码
/// 跳转后立刻读取会拿到空串，表现为「窗口停在个人主页，既不保存凭据也不关闭」。
const COOKIE_SETTLE: Duration = Duration::from_millis(800);

/// 读取 Cookie 的重试次数（每次都先静置 `COOKIE_SETTLE`）。
/// 只重试一次就够：真正的登录态 Cookie 由跳转响应下发，多等一轮即可落盘；
/// 再多轮只会让用户在窗口前多等，解决不了「压根没登录成功」的情况。
const COOKIE_SETTLE_ATTEMPTS: u32 = 2;

/// Rust 侧 Cookie 轮询间隔。
///
/// 51CTO 的登录凭据（`pub_auth_profile` / `pub_sauth1` 等）是 `HttpOnly`，页面内
/// `document.cookie` 永远读不到，DOM 探针的鉴权判定因此恒为 false。唯一的出路是
/// 由 Rust 侧直读 WebView2 底层 Cookie 容器（`cookies_for_url`，能看到 HttpOnly）。
/// 1 秒一次既能保证「打开窗口时本来已登录」的秒关体验，又不至于空转太频繁。
const COOKIE_POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// 轮询次数上限（约 3 分钟）。用户扫码 + 手机确认通常远小于这个时长。
/// 超时不关窗：窗口仍然开着，用户随时可以继续登录；只是不再由轮询兜底。
const COOKIE_POLL_ATTEMPTS: usize = 180;

/// 启用 Rust 侧 Cookie 轮询的平台。
///
/// 只收录判定足够严格的平台：51CTO 走 `fifty_one_cto_logged_in_cookie`，要求
/// `pub_auth_profile` / `pub_cookietime` 的**值**非空非 0，匿名访客必然为空，
/// 因此在登录页上轮询也不会误判；阿里云走 `aliyun_logged_in_cookie`，只认
/// 登录后才写入的 `login_aliyunid_ticket` / `login_aliyunid`；开源中国走
/// `oschina_logged_in_cookie`，只认 `oscid` 等登录票据；百家号走
/// `baijiahao_logged_in_cookie`，只认 `BDUSS` 等百度通行证票据；网易号走
/// `netease_logged_in_cookie`，只认 `NTES_SESS` / `NTES_SESSID`。
///
/// 靠 `AUTH_COOKIE_HINTS` 键名白名单判定的平台**不要**加进来：它们在登录页上
/// 就可能存在匿名 `PHPSESSID` / `session` 之类同名 Cookie，轮询会立刻误判并
/// 秒关窗口，正是本模块反复踩过的坑。新增平台前必须先补一条专属判据。
///
/// 网易号尤其不能靠通用白名单：`NTES_SESS` 含 `sess` 子串会被放行，而网易给
/// 匿名访客下发的会话标记同样含 `sess`，两边在子串匹配下无法区分。
const COOKIE_POLL_PLATFORMS: &[&str] = &["51cto", "aliyun", "oschina", "baijiahao", "netease"];

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
    /// 登录标志 Cookie（简书、51CTO 等），必须靠 DOM 探针确认。
    required_keys: Vec<String>,
    finalized: OnceFlag,
    last_verdict: Mutex<Option<LoginVerdict>>,
}

impl LoginSession {
    fn new(platform: String, required_keys: Vec<String>) -> Self {
        Self {
            platform,
            required_keys,
            finalized: OnceFlag::default(),
            last_verdict: Mutex::new(None),
        }
    }

    /// 没有明确 Cookie 键的平台不能「一开窗就判定已登录」，必须等 DOM 探针确认。
    fn requires_confirmation(&self) -> bool {
        self.required_keys.is_empty()
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
    /// 是否为「确定失败」。
    ///
    /// `true`：后端能确定该平台当前未登录（例如登录窗口被用户关掉且始终
    /// 没读到登录 Cookie），前端可以据此把状态降成「未登录」。
    /// `false`：窗口仍保留、用户可能正在扫码，前端必须保持原状态不动——
    /// 否则刚打开登录窗口就会把已有登录态误标成未登录。
    definitive: bool,
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
///
/// 这是**非确定失败**：只解除前端锁，不改动登录态判定。
pub(crate) fn emit_failed(app: &AppHandle, platform: &str, message: impl Into<String>) {
    let _ = app.emit(
        EVENT_FAILED,
        FailedPayload {
            platform: platform.to_string(),
            message: message.into(),
            definitive: false,
        },
    );
}

/// 确定失败：后端已确认该平台当前没有登录态，前端可以把状态降到「未登录」。
///
/// 与 [`emit_failed`] 的区别只在 `definitive`。调用方必须真的掌握证据
/// （例如通道窗口被关闭且 Cookie 里始终没有登录项），不能凭「等超时」就断言。
pub(crate) fn emit_auth_invalid(app: &AppHandle, platform: &str, message: impl Into<String>) {
    let _ = app.emit(
        EVENT_FAILED,
        FailedPayload {
            platform: platform.to_string(),
            message: message.into(),
            definitive: true,
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
/// 只做保守的正向匹配：host 含 `passport` / `login`，或 **path** 命中登录相关
/// 关键词。命中意味着「这里还没有登录态」，绝不能据此抓 Cookie 收尾。
///
/// 刻意**不看 query**：登录成功后平台常把目标地址塞进查询参数，例如 51CTO 的
/// `home.51cto.com/index?from_service=blog&reback=...login-success...`。把 query
/// 算进来会把这个「已经登录成功」的落地页误判成登录页，探针因此永远不上报，
/// 窗口卡死在登录页，用户怎么扫码都没反应。
pub fn looks_like_login_page(url: &Url) -> bool {
    const HINTS: &[&str] = &["login", "sign_in", "signin", "sign-in", "passport", "sso"];
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if host.contains("passport") || host.contains("login") {
        return true;
    }
    let path = url.path().to_ascii_lowercase();
    // `login-success` / `login_success` 是登录**成功**的落地页，正好与登录页相反。
    if path.contains("login-success") || path.contains("login_success") {
        return false;
    }
    HINTS.iter().any(|hint| path.contains(hint))
}

/// 判断一个地址是否「可能是登录后的页面」。
///
/// 这**只是 URL 层面的粗筛**，不构成登录判据：51CTO 的个人主页
/// `blog.51cto.com/u_{uid}` 匿名也能打开，退出登录后同样会落在这里，仅凭路径
/// 判定会造成「退出反而算成功」的反转（已实测复现）。因此本函数仅用于日志诊断，
/// 真正的抓取触发源只有 DOM 探针（见 `LOGIN_PROBE_SCRIPT`）。
///
/// 判定口径：host 属于该平台、且路径不是登录/注册/授权页。
fn looks_like_logged_in_page(platform: &str, url: &Url) -> bool {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    // 只看 path，不看 query：登录后的跳转常带 `?from=login` 之类的来源参数，
    // 若把 query 也算进来，真实目标页会被 `looks_like_login_page` 误判成登录页。
    if path.contains("login")
        || path.contains("passport")
        || path.contains("register")
        || path.contains("signup")
        || path.contains("oauth")
    {
        return false;
    }
    match platform {
        "51cto" => host.contains("51cto.com"),
        _ => false,
    }
}

/// 登录入口与业务站点不在同一域时，需要额外补抓的地址。
///
/// 阿里云横跨三个域：统一登录入口在 `login.aliyun.com`，登录完成后的账号页在
/// `account.aliyun.com`，而发文章用的是 `developer.aliyun.com`。真正的 SSO 票据
/// （`login_aliyunid_ticket` 等）是 `HttpOnly` 且下发在统一登录域上的，只查业务
/// 域必然读不到，表现为「扫码成功、DOM 探针 hits=1，但 auth=none、窗口卡死」。
/// 因此三个域都要查一遍。
///
/// `c_csrf` 这个防 CSRF 令牌则是**业务子域下发**的 host-only Cookie，只查登录域
/// 同样取不到，表现为「状态显示已登录，但发布时提示 Cookie 中缺少 c_csrf」。
///
/// WebView2 的 `cookies_for_url` 不接受通配域名，只能按具体 URL 查询：传入
/// `developer.aliyun.com` 会同时返回 `.aliyun.com` 的域级 Cookie 与该子域的
/// host-only Cookie，因此列出具体域名即可覆盖「放宽到 .aliyun.com」的诉求。
///
/// 51CTO 同理：登录入口在 `blog.51cto.com`，但登录态落地页常在
/// `home.51cto.com`（`?reback=...login-success...`），凭据也可能下发在
/// `www.51cto.com`。只查登录入口那一侧会漏掉这些子域的 host-only Cookie。
///
/// 开源中国同理：登录入口在 `www.oschina.net/home/login`，但登录票据常在
/// 个人中心 `my.oschina.net` 下发（发布接口 `apiv1.oschina.net` 正是拿这份票据
/// 换会话）。只查登录入口会读不到 `oscid`，表现为「页面已渲染登录特征、
/// 探针 auth=none、窗口卡死」。
///
/// 百家号同理：业务入口在 `baijiahao.baidu.com`，但真正的登录凭据
/// （`BDUSS` / `STOKEN` / `PTOKEN`）由百度通行证 `passport.baidu.com` 下发，
/// 域是 `.baidu.com`。这些票据是 `HttpOnly`，页面内读不到，只查业务域时若
/// 票据恰好是 passport 子域的 host-only Cookie 就会漏掉，因此两处都查。
///
/// 网易号同理：业务入口在 `mp.163.com`，但登录走网易统一通行证。`NTES_SESS`
/// 由通行证下发且是 `HttpOnly`，页面内读不到，只查业务域时若票据是通行证子域的
/// host-only Cookie 就会漏掉，表现为「页面已渲染登录特征、探针 auth=none、窗口卡死」。
/// 因此 `reg.163.com` 与 `passport.163.com` 都要查——通行证在不同登录入口下
/// 可能把票据落在其中任一子域。
fn supplemental_cookie_urls(platform: &str) -> Vec<Url> {
    let raw: &[&str] = match platform {
        "aliyun" => &[
            "https://login.aliyun.com/",
            "https://account.aliyun.com/",
            "https://developer.aliyun.com/",
        ],
        "51cto" => &["https://home.51cto.com/", "https://www.51cto.com/"],
        "oschina" => &["https://my.oschina.net/", "https://www.oschina.net/"],
        "baijiahao" => &["https://passport.baidu.com/", "https://baijiahao.baidu.com/"],
        "netease" => &[
            "https://reg.163.com/",
            "https://passport.163.com/",
            "https://mp.163.com/",
        ],
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

/// 51CTO 专属登录判据：`pub_auth_profile` 是站点下发的身份画像 Cookie。
///
/// 真机实测发现，**未登录**访客也会拿到一整套 `pub_*` Cookie
/// （`pub_auth_profile` / `pub_cookietime` / `pub_wechatopen` / `once_p`），
/// 外加一个匿名 `PHPSESSID`。因此「Cookie 里存在 PHPSESSID」根本证明不了登录，
/// 而 `auth_cookie_key` 会把它当成鉴权字段放行——这正是「保存了凭据却发布未登录」
/// 的根因。
///
/// 真正能区分的是这些 Cookie 的**值**：匿名时是空串或 `0`，登录后才会填入用户
/// 标识/时间戳。这里只认非空且非 `0` 的 `pub_auth_profile` 或非 `0` 的
/// `pub_cookietime`。
fn fifty_one_cto_logged_in_cookie(cookie: &str) -> bool {
    for pair in cookie.split(';') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if value.is_empty() || value == "0" || value == "null" {
            continue;
        }
        if name.eq_ignore_ascii_case("pub_auth_profile")
            || name.eq_ignore_ascii_case("pub_cookietime")
        {
            return true;
        }
    }
    false
}

/// 阿里云专属登录判据：只认「登录成功后才写入」的 SSO 票据。
///
/// 与 51CTO 同因：阿里云的鉴权字段是 `HttpOnly` 的，页面内 `document.cookie`
/// 读不到。真机表现为 DOM 探针 `hits=1` 但 `auth=none`，探针永远等不到确认，
/// 窗口卡在登录页。只能由 Rust 侧读底层 Cookie 容器。
///
/// 键名用**精确匹配**而非子串，且只收录强键名：
/// - `login_aliyunid_ticket`：SSO 票据，扫码成功后由 `login.aliyun.com` 下发；
/// - `login_aliyunid`：账号标识，匿名时不存在或值为空；
/// - `aliyun_session`：会话键名，若存在则值非空即视为登录。
///
/// 刻意**不收** `t` / `cookie2` / `_tb_token_` / `cna` / `xlly_s` / `tfstk` 这类
/// 阿里巴巴系通用字段：它们在匿名访问时同样存在（埋点与风控用途），一旦纳入
/// 判据，轮询会在窗口打开的第一秒命中、秒关窗口并存下一份匿名凭据——正是本
/// 模块反复踩过的坑。若真机日志（`[login] 阿里云轮询 keys=[...]`）显示登录后
/// 只剩 `t`，再按实际输出补收，而不是提前猜。
const ALIYUN_LOGIN_COOKIE_KEYS: &[&str] = &[
    "login_aliyunid_ticket",
    "login_aliyunid",
    "aliyun_session",
];

fn aliyun_logged_in_cookie(cookie: &str) -> bool {
    for pair in cookie.split(';') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if value.is_empty() || value == "0" || value == "null" {
            continue;
        }
        if ALIYUN_LOGIN_COOKIE_KEYS
            .iter()
            .any(|key| name.eq_ignore_ascii_case(key))
        {
            return true;
        }
    }
    false
}

/// 开源中国专属登录判据。
///
/// 与阿里云同因：`oscid` 是 `HttpOnly` 的会话票据，页面内 `document.cookie`
/// 读不到，DOM 探针只能看到 `auth=none`，永远等不到确认信号，窗口卡在登录页。
/// 只能由 Rust 侧读底层 Cookie 容器。
///
/// 键名用**精确匹配**，值必须非空且非 `0` / `null`。
///
/// 注意 `oschina` / `oschina_new_user` 属于**弱信号**：它们在匿名访问时也可能
/// 被下发（站点用来区分新老访客）。真机若出现「窗口一打开就 passed=true」，
/// 日志里的 `[login] 开源中国轮询 keys=[...]` 会立刻显示这两个键在未登录时
/// 就已存在——届时把它们从常量里删掉，只留 `oscid`，而不是放宽判据。
const OSCHINA_LOGIN_COOKIE_KEYS: &[&str] = &["oscid", "oschina_new_user", "oschina"];

fn oschina_logged_in_cookie(cookie: &str) -> bool {
    for pair in cookie.split(';') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if value.is_empty() || value == "0" || value == "null" {
            continue;
        }
        if OSCHINA_LOGIN_COOKIE_KEYS
            .iter()
            .any(|key| name.eq_ignore_ascii_case(key))
        {
            return true;
        }
    }
    false
}

/// 百家号专属登录判据。
///
/// `BDUSS` 是百度通行证的登录会话票据（`HttpOnly`，域 `.baidu.com`），页面内
/// `document.cookie` 读不到，DOM 探针只能看到 `auth=none`。`STOKEN` / `PTOKEN`
/// 是同一套通行证下发的接口票据，登录后才有。
///
/// 键名用**精确匹配**，值必须非空且非 `0` / `null`。
///
/// 刻意**不收** `BAIDUID` / `BAIDUID_BFESS` / `BIDUPSID` / `PSTM` / `H_PS_PSSID`：
/// 这些是百度匿名访客就带的全量标识与统计字段，纳入判据会让窗口一打开就误判。
/// 尤其 `BIDUPSID` 含 `sid` 子串，通用白名单的 `sid_token` 之类写法也有同类风险，
/// 因此百家号必须走专属判据而非 `AUTH_COOKIE_HINTS`。
const BAIJIAHAO_LOGIN_COOKIE_KEYS: &[&str] = &[
    "BDUSS",
    "BDUSS_BFESS",
    "STOKEN",
    "PTOKEN",
];

fn baijiahao_logged_in_cookie(cookie: &str) -> bool {
    for pair in cookie.split(';') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if value.is_empty() || value == "0" || value == "null" {
            continue;
        }
        if BAIJIAHAO_LOGIN_COOKIE_KEYS
            .iter()
            .any(|key| name.eq_ignore_ascii_case(key))
        {
            return true;
        }
    }
    false
}

/// 网易号专属登录判据。
///
/// `NTES_SESS` 是网易统一通行证下发的登录会话票据（`HttpOnly`，域 `.163.com`），
/// 页面内 `document.cookie` 读不到，DOM 探针只能看到 `auth=none`。它是网易号
/// **唯一**可信的强信号。
///
/// 键名用**精确匹配**，值必须非空且非 `0` / `null`。
///
/// 网易号**必须**走专属判据而不能用 `AUTH_COOKIE_HINTS`：通用白名单里的 `sess`
/// 子串会放行 `NTES_SESS`，但网易给匿名访客下发的会话标记同样含 `sess`，子串
/// 匹配下两者无法区分，轮询会在登录页上就误判。
///
/// 为什么**不能**收 `P_INFO`（真机日志已定案，不要"顺手补回来"）：
/// 未登录状态下轮询打印的是 `keys=[NTESwebSI,P_INFO,_antanalysis_s_id,_ntes_nuid]`，
/// `P_INFO` 在匿名访客身上**存在且带值**，于是判定直接 `passed=true`，窗口在
/// 用户还没扫码时就被关掉、存下一份匿名凭据。
///
/// 这与阿里云的 `_tb_token_`、开源中国的 `oschina_new_user` 是**同一类错误**：
/// 名字看起来像"用户信息 / 会话"，实际上匿名也下发、且带非空值。教训一致——
/// **判断一个键能否当判据，看的是"未登录时有没有值"，不是名字像不像**。
/// 因此 `P_INFO` / `S_INFO` / `NTESwebSI` / `_ntes_nuid` / `_antanalysis_s_id`
/// / `_ntes_origin_from` / `_ntes_nnid` / `NEGID` / `NTES_P_UTID` 一律不收。
///
/// 主信号是 `NTES_YD_SESS`：网易号（`mp.163.com`）自己的会话票据，扫码登录成功
/// 后才下发，匿名访问时不存在。判据来源是**真机日志实证**，不是名字猜测——
/// 登录前 Cookie 220 字节、登录后 512 字节，差值就是这张票据。
///
/// `NTES_SESS` / `NTES_SESSID` 是网易通行证主站（`passport.163.com`）的会话键名，
/// 与网易号业务域的票据不是一回事。保留它们**只作兜底**：某些登录流程下主站会话
/// 可能同时出现。不要因为「名字更像 session」就把它们提到前面，否则下一次校正
/// 键名时又会把主站键名当成网易号的主信号。
///
/// 补充一条反例：日志里 `S_INFO` 也是登录后才出现，但它与 `P_INFO` 同族、语义不
/// 明确（匿名链路同样会写），因此不进判据。宁可少一个信号，也不要多一次误判。
const NETEASE_LOGIN_COOKIE_KEYS: &[&str] =
    &["NTES_YD_SESS", "NTES_SESS", "NTES_SESSID"];

fn netease_logged_in_cookie(cookie: &str) -> bool {
    for pair in cookie.split(';') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if value.is_empty() || value == "0" || value == "null" {
            continue;
        }
        if NETEASE_LOGIN_COOKIE_KEYS
            .iter()
            .any(|key| name.eq_ignore_ascii_case(key))
        {
            return true;
        }
    }
    false
}

/// 鉴权字段白名单（键名子串匹配，大小写不敏感）。
///
/// 只有命中这些名字的 Cookie 才算「真的登录了」。51CTO / 简书等站点的会话
/// Cookie 常带 `token` / `session` / `userid` / `auth`；而统计与来源标记 Cookie
/// （`Hm_*`、`_ga`、`CNZZ*`、`BIDUPSID`、`acw_tc`、`login_from`）匿名访问同样
/// 存在，绝不能作为登录证据。
///
/// 刻意**不收录** `sid` / `uid` / `login` 这类短词或弱词：`BIDUPSID`（百度统计）
/// 含 `sid`，`login_from` 是记录来源页的普通字段、含 `login`，写成子串匹配会把
/// 匿名 Cookie 当成登录态放进来（真机日志已实测出现 `auth=login_from`）。
const AUTH_COOKIE_HINTS: &[&str] = &[
    "token",
    "session",
    "sessid",
    "sess",
    "userid",
    "user_id",
    "passport",
    "access_token",
    "auth",
    "ssotk",
    "sid_token",
    "51cto",
];

/// 从 Cookie 串里找出第一个命中鉴权白名单的键名，供日志与校验使用。
fn auth_cookie_key(cookie: &str) -> Option<String> {
    cookie.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.is_empty() {
            return None;
        }
        let lower = name.to_ascii_lowercase();
        AUTH_COOKIE_HINTS
            .iter()
            .any(|hint| lower.contains(hint))
            .then(|| name.to_string())
    })
}

/// 需要在轮询里逐轮打印 Cookie 键名清单的平台，返回展示用的平台名。
///
/// 只收录「鉴权键名不确定、需靠真机日志校正」的平台：多打一行是排查成本最低
/// 的手段，键名清单能直接区分「域名没覆盖到」与「键名判断错」两种失败。
fn cookie_key_log_label(platform: &str) -> Option<&'static str> {
    match platform {
        "aliyun" => Some("阿里云"),
        "oschina" => Some("开源中国"),
        "baijiahao" => Some("百家号"),
        "netease" => Some("网易号"),
        _ => None,
    }
}

/// 从 Cookie 串里取出键名（已排序、已去空），供日志使用。不取值。
fn cookie_key_names(cookie: &str) -> Vec<&str> {
    let mut names: Vec<&str> = cookie
        .split(';')
        .filter_map(|pair| pair.split_once('=').map(|(name, _)| name.trim()))
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_unstable();
    names
}

/// Rust 侧 Cookie 轮询：直读 WebView2 底层 Cookie 容器，**能读到 `HttpOnly`**。
///
/// 为什么必须存在：51CTO 的登录凭据（`pub_auth_profile` / `pub_sauth1` 等）是
/// `HttpOnly`，页面内 `document.cookie` 永远读不到，DOM 探针的 `auth` / `cto`
/// 判定因此恒为 false，探针这条路对 51CTO 是死路。真机已验证：同一份 Cookie
/// 手动填进配置能正常发草稿，说明凭据确实在系统 Cookie 容器里，只是 JS 看不见。
///
/// 另一个收益是「打开窗口时本来就已经登录」的场景：探针要等页面 DOM 出现登录
/// 特征才可能 tick 成功，而轮询在窗口创建后 1 秒内就能直接读到凭据并收尾，
/// 用户不用等页面渲染完，体验上就是「秒关窗」。
///
/// 与探针的关系是**并行兜底**，不是替代：两边都走 `has_really_logged_in`，
/// 结论一致；谁先通过谁收尾，`finalized` 保证只收尾一次。
fn spawn_cookie_poller(window: WebviewWindow, urls: Vec<Url>, session: Arc<LoginSession>) {
    let platform = session.platform.clone();
    let label = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        for attempt in 1..=COOKIE_POLL_ATTEMPTS {
            std::thread::sleep(COOKIE_POLL_INTERVAL);
            // 探针可能已经收尾；窗口也可能已被用户关掉。两种情况都直接退出。
            if session.finalized.is_set() {
                return;
            }
            if window.app_handle().get_webview_window(&label).is_none() {
                eprintln!(
                    "[login] Cookie 轮询退出：窗口已关闭 platform={} attempt={}",
                    platform, attempt
                );
                return;
            }
            let cookie_string = match collect_cookie_string(&window, &urls) {
                Ok(cookie) => cookie,
                Err(message) => {
                    eprintln!(
                        "[login] Cookie 轮询读取失败 platform={} attempt={} err={}",
                        platform, attempt, message
                    );
                    continue;
                }
            };
            let passed = has_really_logged_in(&platform, &cookie_string);
            // 这些平台的鉴权键名在真机上并不稳定（不同子域下发的名字不一样），
            // 逐轮打印键名清单，便于按实际输出校正各自的 `*_LOGIN_COOKIE_KEYS`。
            // 只打键名，不打值。
            if let Some(label) = cookie_key_log_label(&platform) {
                eprintln!(
                    "[login] {}轮询 keys=[{}]",
                    label,
                    cookie_key_names(&cookie_string).join(",")
                );
            }
            let page_url = window
                .url()
                .map(|u| u.to_string())
                .unwrap_or_else(|_| "<unknown>".to_string());
            eprintln!(
                "[login] Cookie 轮询 {}/{} platform={} page_url={} len={} passed={}",
                attempt,
                COOKIE_POLL_ATTEMPTS,
                platform,
                page_url,
                cookie_string.len(),
                passed
            );
            if passed {
                eprintln!(
                    "[login] Cookie 轮询命中登录态，立即收尾 platform={} page_url={}",
                    platform, page_url
                );
                capture_and_finalize(window, urls, session);
                return;
            }
        }
        eprintln!(
            "[login] Cookie 轮询达到上限，交由 DOM 探针继续等待 platform={}",
            platform
        );
    });
}

/// 确认登录态后抓取全量 Cookie 并收尾。
///
/// 调用方有两处：DOM 探针回传 `LOGIN_OK_MARKER`，以及 Rust 侧 Cookie 轮询
/// （见 `spawn_cookie_poller`，用于 HttpOnly 凭据读不到的场景）。这里仍要独立
/// 校验鉴权字段，因为「页面 DOM 出现登录特征」与「Cookie 里真有会话」在时间上
/// 并不严格同步，且退出登录后页面可能残留头像节点。没有鉴权字段就拒绝保存——
/// 否则会存下一份匿名 Cookie，前端显示「已登录」而发布必然失败。
fn capture_and_finalize(window: WebviewWindow, urls: Vec<Url>, session: Arc<LoginSession>) {
    if session.finalized.is_set() {
        return;
    }
    let platform = session.platform.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let urls = with_supplemental_urls(&platform, urls);
        let mut cookie_string = String::new();
        // 先静置再读：登录态 Cookie 由跳转响应的 `Set-Cookie` 下发，WebView2
        // 落盘与本进程读取之间有毫秒级时间差，读到空串就再等一轮。
        for attempt in 1..=COOKIE_SETTLE_ATTEMPTS {
            std::thread::sleep(COOKIE_SETTLE);
            cookie_string = match collect_cookie_string(&window, &urls) {
                Ok(cookie) => cookie,
                Err(message) => {
                    if session.finalized.try_acquire() {
                        emit_failed(window.app_handle(), &platform, message);
                    }
                    return;
                }
            };
            if has_really_logged_in(&platform, &cookie_string) {
                break;
            }
            eprintln!(
                "[login] 第 {}/{} 次读取未发现实际登录凭据 platform={}",
                attempt, COOKIE_SETTLE_ATTEMPTS, platform
            );
        }

        let page_url = window
            .url()
            .map(|u| u.to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        let still_on_login = window
            .url()
            .map(|u| looks_like_login_page(&u))
            .unwrap_or(false);
        eprintln!(
            "[login] capture_and_finalize platform={} page_url={} cookie_len={} auth_key={:?}",
            platform,
            page_url,
            cookie_string.len(),
            auth_cookie_key(&cookie_string)
        );

        // 兜底闸门：探针理论上不会在登录页上报，但页面可能在探针 tick 与 Rust
        // 读取之间又跳回登录页（例如登录失败重试）。此时抓到的必然是匿名 Cookie，
        // 一律拒绝，窗口保持打开让用户继续登录。
        //
        // 例外：**Cookie 已经通过真实鉴权校验时不再看 URL**。51CTO 登录成功后
        // 常常原地停在 `/login`（页面自身重渲染，URL 不变），此时凭据已经在
        // Cookie 容器里，按 URL 拒绝会把一次成功的登录判成失败。
        if still_on_login && !has_really_logged_in(&platform, &cookie_string) {
            session.record(LoginVerdict::Cancelled);
            eprintln!(
                "[login] 拒绝保存：当前页面仍是登录页且无有效凭据 platform={} url={}",
                platform, page_url
            );
            emit_failed(
                window.app_handle(),
                &platform,
                "当前仍停留在登录页，请先在窗口内完成登录",
            );
            return;
        }

        if cookie_string.trim().is_empty() {
            session.record(LoginVerdict::Cancelled);
            // 不能静默返回：那样窗口会一直悬在平台页面上，用户只会觉得程序卡住。
            // 窗口保持打开，方便用户重试；前端据此解除「登录中」状态。
            emit_failed(
                window.app_handle(),
                &platform,
                "已确认登录跳转，但未读取到 Cookie，请关闭窗口后重新登录",
            );
            return;
        }

        // 有 Cookie 不代表登录：统计 Cookie、匿名访客标识同样一堆。
        // 51CTO 需要额外校验专属身份字段，否则匿名 `PHPSESSID` +
        // `pub_auth_profile`（空值）会被当成登录态存下来，发布时才报未登录。
        if !has_really_logged_in(&platform, &cookie_string) {
            session.record(LoginVerdict::MissingKeys(vec!["鉴权字段".to_string()]));
            eprintln!(
                "[login] 拒绝保存：Cookie 中不含实际登录凭据 platform={} keys=[{}]",
                platform,
                cookie_string
                    .split(';')
                    .filter_map(|pair| pair.split_once('=').map(|(n, _)| n.trim()))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            emit_failed(
                window.app_handle(),
                &platform,
                "页面显示已登录，但 Cookie 中没有鉴权字段，尚未完成登录，请稍候或重新登录",
            );
            return;
        }

        finalize_login(window, session, cookie_string);
    });
}

/// 判定抓到的 Cookie 是否真的代表登录态。
///
/// 通用平台走 `AUTH_COOKIE_HINTS` 键名白名单；51CTO、阿里云、开源中国、百家号、
/// 网易号例外——它们的匿名访客同样带着一整套埋点 / 风控 Cookie，只有值被填过的
/// 专属字段才说明真登录，因此分别走各自专属判据。
fn has_really_logged_in(platform: &str, cookie: &str) -> bool {
    match platform {
        "51cto" => fifty_one_cto_logged_in_cookie(cookie),
        "aliyun" => aliyun_logged_in_cookie(cookie),
        "oschina" => oschina_logged_in_cookie(cookie),
        "baijiahao" => baijiahao_logged_in_cookie(cookie),
        "netease" => netease_logged_in_cookie(cookie),
        _ => auth_cookie_key(cookie).is_some(),
    }
}

/// 页面加载完成后的判定入口，两条路径泾渭分明：
///
/// - 有明确登录标志 Cookie 的平台：照旧按 key 判定，缺 key 只记录不关窗；
/// - 没有明确登录标志的平台（简书、51CTO 等）：**只注入 DOM 探针**，抓取与收尾
///   完全由探针回传的 `LOGIN_OK_MARKER` 触发。
///
/// 这里刻意不再有「见过登录页 + 跳走 = 登录成功」这条判据。它曾被用来兜底
/// 「登录入口认不出来」的平台，但用户退出登录时同样会首页 → 登录页 → 首页，
/// `seen_login_page` 一旦置位就永久为真，第二次加载直接抓取并保存了一份匿名
/// Cookie，表现为「点了退出反而提示登录成功、同步时才报未登录」（已实测复现）。
/// 离开登录页只能说明「用户不再停在登录页」，不能说明「他已登录」。
fn handle_page_load(
    window: WebviewWindow,
    page_url: Url,
    cookie_url: Url,
    session: Arc<LoginSession>,
) {
    eprintln!(
        "[login] handle_page_load platform={} url={} finalized={} requires_confirmation={}",
        session.platform,
        page_url.as_str(),
        session.finalized.is_set(),
        session.requires_confirmation(),
    );
    if session.finalized.is_set() {
        eprintln!("[login]   → 已 finalized，跳过");
        return;
    }

    if !session.requires_confirmation() {
        let required_keys = session.required_keys.clone();
        eprintln!("[login]   → 有明确 Cookie 键，走 extract_by_keys");
        extract_by_keys(window, cookie_url, required_keys, session);
        return;
    }

    // 登录页与普通页都要注入 DOM 探针：登录成功常常是页面内跳转或原页刷新，
    // 注入后它自己会持续 tick，不必依赖下一次 on_page_load。探针内部有三层
    // 判定（非登录页 + DOM 登录特征 + 鉴权 Cookie），登录页上它自然什么都不做。
    if looks_like_login_page(&page_url) {
        eprintln!("[login]   → 命中登录页，注入 DOM 探针（不抓 Cookie）");
    } else {
        eprintln!("[login]   → 非登录页，注入 DOM 探针，等待登录特征 + 鉴权 Cookie 同时成立");
    }
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
    // 与通道窗口共用同一个 WebView2 profile：否则登录窗口抓到的 Cookie 落在
    // 另一个数据目录里，通道窗口完全看不到，表现为「登录成功但同步仍要求登录」。
    let data_dir = webview::webview_data_dir(&app)?;
    let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
        .title(format!("{} - 登录", platform))
        .inner_size(900.0, 720.0)
        .data_directory(data_dir);
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
            if title == LOGIN_OK_MARKER {
                let page_url = window
                    .url()
                    .map(|u| u.to_string())
                    .unwrap_or_else(|_| "<unknown>".to_string());
                eprintln!("[login] DOM 探针命中，page_url={}，开始抓 Cookie", page_url);
                let mut urls = vec![title_cookie_url.clone()];
                if let Ok(current) = window.url() {
                    if current.as_str() != title_cookie_url.as_str() {
                        urls.push(current);
                    }
                }
                capture_and_finalize(window, urls, Arc::clone(&title_session));
                return;
            }
            if title.starts_with(PROBE_NO_AUTH_PREFIX) {
                eprintln!(
                    "[login] DOM 探针认出登录特征但不含鉴权 Cookie，继续等待: {}",
                    title
                );
                return;
            }
            if title.starts_with(PROBE_PENDING_PREFIX) {
                eprintln!("[login] DOM 探针 tick: {}", title);
                return;
            }
            // 页面自己的 title（不是探针写的），忽略
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

    // 对「凭据是 HttpOnly、JS 读不到」的平台启动 Rust 侧轮询兜底。
    // 窗口一建好就开始读，用户本来已登录的话 1 秒内就会收尾关窗。
    if COOKIE_POLL_PLATFORMS.contains(&session.platform.as_str()) {
        let poll_urls = with_supplemental_urls(&session.platform, vec![cookie_url]);
        eprintln!(
            "[login] 启动 Cookie 轮询 platform={} urls={}",
            session.platform,
            poll_urls
                .iter()
                .map(|u| u.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
        spawn_cookie_poller(window.clone(), poll_urls, Arc::clone(&session));
    }

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
                   "baijiahao", "netease", "bilibili", "weibo"] {
            assert!(ensure_login_supported(id).is_ok(), "{} 应支持一键登录", id);
        }
        // 知乎、简书、头条的登录走各自的通道窗口，不走 `login-{platform}`，因此这里必须拒绝。
        for id in ["zhihu", "jianshu", "toutiao"] {
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
    fn supplemental_urls_cover_51cto_sibling_subdomains() {
        // 51CTO 登录落地页常在 home.51cto.com，凭据可能下发在 www.51cto.com。
        // 只查登录入口 blog.51cto.com 会漏掉这些子域的 host-only Cookie。
        let extra: Vec<String> = supplemental_cookie_urls("51cto")
            .iter()
            .map(|u| u.as_str().to_string())
            .collect();
        assert!(extra.contains(&"https://home.51cto.com/".to_string()));
        assert!(extra.contains(&"https://www.51cto.com/".to_string()));

        let merged = with_supplemental_urls(
            "51cto",
            vec![Url::parse("https://blog.51cto.com/login").unwrap()],
        );
        assert_eq!(merged.len(), 3, "登录入口 + 两个兄弟子域");
        // 去重：同一地址不能重复抓两遍。
        let dup = with_supplemental_urls(
            "51cto",
            vec![
                Url::parse("https://blog.51cto.com/login").unwrap(),
                Url::parse("https://home.51cto.com/").unwrap(),
            ],
        );
        assert_eq!(dup.len(), 3);
    }

    #[test]
    fn cookie_poll_only_enabled_for_strict_verdict_platforms() {
        // 轮询只对判定足够严格的平台启用。靠键名白名单的平台在登录页上就有
        // 匿名 PHPSESSID / session，轮询会秒判并误关窗口。
        assert_eq!(
            COOKIE_POLL_PLATFORMS,
            &["51cto", "aliyun", "oschina", "baijiahao", "netease"]
        );
        // 白名单里的平台必须各自有专属判据，否则等于把宽松判据塞进轮询。
        assert!(has_really_logged_in("51cto", "pub_auth_profile=uid_1"));
        assert!(has_really_logged_in("aliyun", "login_aliyunid_ticket=T"));
        assert!(has_really_logged_in("oschina", "oscid=abc"));
        assert!(has_really_logged_in("baijiahao", "BDUSS=abc"));
        assert!(has_really_logged_in("netease", "NTES_SESS=abc"));
        for platform in ["jianshu", "juejin", "csdn", "zhihu", "weibo"] {
            assert!(
                !COOKIE_POLL_PLATFORMS.contains(&platform),
                "白名单判定平台不得启用轮询: {}",
                platform
            );
        }
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

        // 51CTO 这类没有登录标志 Cookie 的平台，必须等 DOM 探针确认。
        let loose = LoginSession::new("51cto".to_string(), Vec::new());
        assert!(loose.requires_confirmation());
    }

    #[test]
    fn login_page_detection_ignores_query_and_success_landing() {
        // 51CTO 登录成功后落地在这个地址，query 里带着 `login-success`。
        // 必须判成「不是登录页」，否则探针永远不上报、窗口卡死。
        for raw in [
            "https://home.51cto.com/index?from_service=blog&reback=https%253A%252F%252Fblog.51cto.com%252Fuser%252Flogin-success%253Fsign%253Dabc",
            "https://blog.51cto.com/user/login-success?sign=abc",
            "https://blog.51cto.com/u_12345",
        ] {
            assert!(
                !looks_like_login_page(&Url::parse(raw).unwrap()),
                "不应识别为登录页: {}",
                raw
            );
        }
        // 真正的登录页仍然要拦住。
        assert!(looks_like_login_page(
            &Url::parse("https://blog.51cto.com/login").unwrap()
        ));
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
    fn logged_in_path_rule_is_only_a_url_coarse_filter() {
        // 这是 URL 粗筛，不是登录判据：51CTO 域下非登录/注册/授权页都算「可能在登录后」。
        for raw in [
            "https://blog.51cto.com/",
            "https://blog.51cto.com/u_1234567",
            "https://blog.51cto.com/blogger/publish",
            "https://blog.51cto.com/blogger/draft",
        ] {
            assert!(
                looks_like_logged_in_page("51cto", &Url::parse(raw).unwrap()),
                "51CTO 非登录页应通过 URL 粗筛: {}",
                raw
            );
        }

        // 登录 / 注册 / 授权页一律不通过。
        for raw in [
            "https://blog.51cto.com/login",
            "https://home.51cto.com/login",
            "https://blog.51cto.com/register",
            "https://blog.51cto.com/signup",
            "https://blog.51cto.com/oauth/authorize",
        ] {
            assert!(
                !looks_like_logged_in_page("51cto", &Url::parse(raw).unwrap()),
                "登录类页面不应通过粗筛: {}",
                raw
            );
        }

        // 别的域名不算，避免误判到第三方站点。
        assert!(!looks_like_logged_in_page(
            "51cto",
            &Url::parse("https://example.com/u_1234567").unwrap()
        ));

        // 其余平台没有这条判据，必须保持 false（否则会提前抓匿名 Cookie）。
        for platform in ["juejin", "oschina", "weibo", "baijiahao"] {
            assert!(!looks_like_logged_in_page(
                platform,
                &Url::parse("https://blog.51cto.com/u_1234567").unwrap()
            ));
        }
    }

    #[test]
    fn auth_cookie_key_accepts_real_session_fields() {
        for (cookie, expected) in [
            ("SESSIONID=abc; uid=42", "SESSIONID"),
            ("acw_tc=x; access_token=tk; _ga=GA1", "access_token"),
            ("token=abcd", "token"),
            ("userid=77", "userid"),
            ("sso_session=zzz", "sso_session"),
            ("AUTH_TOKEN=xx", "AUTH_TOKEN"),
        ] {
            let key = auth_cookie_key(cookie);
            assert_eq!(key.as_deref(), Some(expected), "cookie={}", cookie);
        }
    }

    #[test]
    fn auth_cookie_key_rejects_analytics_only_cookies() {
        // 匿名访问常见的一堆统计 Cookie，一个都不能当成登录证据。
        // BIDUPSID 含 sid、login_from 含 login，正是白名单刻意不收短词/弱词的原因。
        // `login_from` 那条来自 51CTO 真机日志：它曾被误判成鉴权字段。
        for cookie in [
            "Hm_lvt_abc=1; Hm_lpvt_abc=1",
            "_ga=GA1.2.3; _gid=GA1.2.4",
            "CNZZDATA123=xyz; BIDUPSID=abc",
            "acw_tc=1; cna=2; _uab_collina=3",
            "login_from=https%3A%2F%2Fblog.51cto.com%2F",
            "uid=42",
            "",
            "token=",
        ] {
            assert!(
                auth_cookie_key(cookie).is_none(),
                "统计类/来源标记 Cookie 不应通过鉴权校验: {}",
                cookie
            );
        }
    }

    #[test]
    fn probe_script_embeds_marker_as_json_literal() {
        let script = login_probe_script();
        assert!(script.contains(&format!("\"{}\"", LOGIN_OK_MARKER)));
        assert!(script.contains(&format!("\"{}\"", PROBE_PENDING_PREFIX)));
        assert!(script.contains(&format!("\"{}\"", PROBE_NO_AUTH_PREFIX)));
        assert!(!script.contains("__MARKER__"), "占位符必须被替换");
        assert!(!script.contains("__PENDING__"), "占位符必须被替换");
        assert!(!script.contains("__NOAUTH__"), "占位符必须被替换");
    }

    #[test]
    fn fifty_one_cto_requires_non_empty_identity_cookie() {
        // 真机日志里的匿名 Cookie 原样：全套 pub_* + 匿名 PHPSESSID，绝不能通过。
        let anonymous = "HMACCOUNT=X; Hm_lpvt_x=1; Hm_lvt_x=1; PHPSESSID=anon;\
             callback_api_url=; cssLoaded=; login_from=x; once_p=1;\
             pub_auth_profile=; pub_cookietime=0; pub_wechatopen=; reg_from=x;\
             sensorsdata2015jssdkcross=y; www51cto=1";
        assert!(!fifty_one_cto_logged_in_cookie(anonymous));

        // 登录后 pub_auth_profile 被填入真实标识 / pub_cookietime 有值。
        assert!(fifty_one_cto_logged_in_cookie(
            "PHPSESSID=abc; pub_auth_profile=uid_123; pub_cookietime=1700000000"
        ));
        assert!(fifty_one_cto_logged_in_cookie(
            "pub_cookietime=1700000000; PHPSESSID=abc"
        ));
        // 只有 pub_auth_profile 填了也算。
        assert!(fifty_one_cto_logged_in_cookie(
            "pub_auth_profile=%7B%22uid%22%3A123%7D; PHPSESSID=abc"
        ));
    }

    #[test]
    fn cookie_with_oschina_oscid_is_logged_in() {
        // 登录后 `my.oschina.net` 下发的 HttpOnly 会话票据。
        assert!(has_really_logged_in(
            "oschina",
            "oscid=abc123; _user_locale=zh; oschina_new_user=false"
        ));
        assert!(oschina_logged_in_cookie("oscid=abc123"));
        // 弱信号键名同样算（真机若证明匿名即存在，需从常量里删掉）。
        assert!(oschina_logged_in_cookie("oschina=1"));
        assert!(oschina_logged_in_cookie("oschina_new_user=true"));
        // 键名大小写不敏感。
        assert!(oschina_logged_in_cookie("OSCID=abc"));
    }

    #[test]
    fn oschina_anonymous_cookies_are_not_logged_in() {
        // 匿名访客：只有埋点 / 语言 / 来源标记，票据缺失或为空。
        let anonymous = "Hm_lvt_x=1; _user_locale=zh; oschina_anonymous=1;\
             oscid=; oschina_new_user=";
        assert!(!oschina_logged_in_cookie(anonymous));
        assert!(!has_really_logged_in("oschina", anonymous));
        // 值为 0 / null 不算。
        assert!(!oschina_logged_in_cookie("oscid=0"));
        assert!(!oschina_logged_in_cookie("oscid=null"));
    }

    #[test]
    fn supplemental_urls_cover_oschina_profile_subdomain() {
        // 登录入口在 www.oschina.net，票据常在 my.oschina.net 下发，两处都要查。
        let extra: Vec<String> = supplemental_cookie_urls("oschina")
            .iter()
            .map(|u| u.as_str().to_string())
            .collect();
        assert!(extra.contains(&"https://my.oschina.net/".to_string()));
        assert!(extra.contains(&"https://www.oschina.net/".to_string()));

        let merged = with_supplemental_urls(
            "oschina",
            vec![Url::parse("https://www.oschina.net/home/login").unwrap()],
        );
        // 去重是按完整 URL 字符串比对，登录页 `/home/login` 与补充域 `/` 不同路径，
        // 因此三个地址都会查。同域重复查询只是多读一次 Cookie，`assemble_cookie_string`
        // 会按同名保留最后一个值，不会产生重复键。
        assert_eq!(merged.len(), 3);
        let unique: std::collections::HashSet<&str> =
            merged.iter().map(|u| u.as_str()).collect();
        assert_eq!(unique.len(), merged.len(), "补充地址不得重复");
    }

    #[test]
    fn cookie_with_baijiahao_bduss_is_logged_in() {
        // 登录后 `passport.baidu.com` 下发的 HttpOnly 通行证票据。
        assert!(has_really_logged_in(
            "baijiahao",
            "BDUSS=abc123; STOKEN=def; BAIDUID=anon"
        ));
        assert!(baijiahao_logged_in_cookie("BDUSS=abc123"));
        assert!(baijiahao_logged_in_cookie("STOKEN=def"));
        assert!(baijiahao_logged_in_cookie("PTOKEN=ghi"));
        // 键名大小写不敏感。
        assert!(baijiahao_logged_in_cookie("bduss=abc"));
    }

    #[test]
    fn baijiahao_anonymous_cookies_are_not_logged_in() {
        // 匿名访客：百度全量匿名标识与统计字段，通行证票据缺失或为空。
        let anonymous = "BAIDUID=anon:FG=1; BAIDUID_BFESS=anon:FG=1;\
             BIDUPSID=anon; PSTM=1700000000; H_PS_PSSID=12345; BDUSS=; STOKEN=";
        assert!(!baijiahao_logged_in_cookie(anonymous));
        assert!(!has_really_logged_in("baijiahao", anonymous));
        // 值为 0 / null 不算。
        assert!(!baijiahao_logged_in_cookie("BDUSS=0"));
        assert!(!baijiahao_logged_in_cookie("BDUSS=null"));
    }

    #[test]
    fn supplemental_urls_cover_baijiahao_passport_domain() {
        // 业务入口在 baijiahao.baidu.com，票据由 passport.baidu.com 下发。
        let extra: Vec<String> = supplemental_cookie_urls("baijiahao")
            .iter()
            .map(|u| u.as_str().to_string())
            .collect();
        assert!(extra.contains(&"https://passport.baidu.com/".to_string()));
        assert!(extra.contains(&"https://baijiahao.baidu.com/".to_string()));

        let merged = with_supplemental_urls(
            "baijiahao",
            vec![Url::parse("https://baijiahao.baidu.com/").unwrap()],
        );
        // 去重按完整 URL 字符串，业务域 `/` 与补充域 `/` 完全相同，会被合并。
        assert_eq!(merged.len(), 2, "登录入口 + 通行证域");
        let unique: std::collections::HashSet<&str> =
            merged.iter().map(|u| u.as_str()).collect();
        assert_eq!(unique.len(), merged.len(), "补充地址不得重复");
    }

    #[test]
    fn cookie_with_netease_session_is_logged_in() {
        // 真机日志实证：网易号（mp.163.com）扫码后下发的是 NTES_YD_SESS，匿名
        // 访问时不存在；NTES_SESS 是网易通行证主站的键名，仅作兜底。
        assert!(has_really_logged_in(
            "netease",
            "NTES_YD_SESS=abc123; P_INFO=user@163.com|1700000000; _ntes_nuid=anon"
        ));
        // 业务域主信号：登录后才有，必须通过。
        assert!(netease_logged_in_cookie("NTES_YD_SESS=xxx"));
        // 主站会话键名：作为兜底，同样通过。
        assert!(netease_logged_in_cookie("NTES_SESS=xxx"));
        assert!(netease_logged_in_cookie("NTES_SESSID=def"));
        // 键名大小写不敏感。
        assert!(netease_logged_in_cookie("ntes_yd_sess=abc"));
        // P_INFO 名字像"用户信息"，但未登录时同样带值，绝不能单独判登录。
        assert!(!netease_logged_in_cookie("P_INFO=xxx"));
    }

    #[test]
    fn netease_anonymous_cookies_are_not_logged_in() {
        // 真机日志定案的匿名键名清单：未登录时这些键全部存在且带值。
        let anonymous = "NTESwebSI=anon; P_INFO=anon|1700000000;\
             _antanalysis_s_id=anon; _ntes_nuid=anon; _ntes_nnid=anon;\
             _ntes_origin_from=anon; NEGID=anon; NTES_P_UTID=anon";
        assert!(!netease_logged_in_cookie(anonymous));
        assert!(!has_really_logged_in("netease", anonymous));
        // 值为 0 / null 不算。
        assert!(!netease_logged_in_cookie("NTES_SESS=0"));
        assert!(!netease_logged_in_cookie("NTES_SESS=null"));
    }

    #[test]
    fn netease_anonymous_key_names_are_never_predicates() {
        // 回归锁：这组键名在未登录时就有值，任何一个被加回判据都会让轮询
        // 在窗口打开的第一秒误判。真机日志 keys=[NTESwebSI,P_INFO,
        // _antanalysis_s_id,_ntes_nuid] → passed=true 即由此而来。
        for weak in [
            "P_INFO",
            "S_INFO",
            "NTESwebSI",
            "_ntes_nuid",
            "_ntes_nnid",
            "_antanalysis_s_id",
            "_ntes_origin_from",
            "NEGID",
            "NTES_P_UTID",
            "NTES_SESS2",
        ] {
            assert!(
                !NETEASE_LOGIN_COOKIE_KEYS
                    .iter()
                    .any(|key| key.eq_ignore_ascii_case(weak)),
                "弱键名不得作为登录判据: {}",
                weak
            );
            assert!(
                !netease_logged_in_cookie(&format!("{}=non_empty", weak)),
                "弱键名不得触发登录判定: {}",
                weak
            );
        }
        // 强信号必须仍在判据里：NTES_YD_SESS 是网易号业务域的主信号，
        // NTES_SESS / NTES_SESSID 是主站兜底。
        assert!(NETEASE_LOGIN_COOKIE_KEYS.contains(&"NTES_YD_SESS"));
        assert!(NETEASE_LOGIN_COOKIE_KEYS.contains(&"NTES_SESS"));
        assert!(NETEASE_LOGIN_COOKIE_KEYS.contains(&"NTES_SESSID"));
    }

    #[test]
    fn netease_needs_dedicated_predicate_because_generic_hints_match() {
        // 通用白名单里的 `sess` 子串会放行匿名会话标记，这正是网易号必须
        // 走专属判据的原因：同一个键名在子串匹配下无法区分登录与匿名。
        let anonymous = "NTES_OSESS=anon";
        assert!(
            auth_cookie_key(anonymous).is_some(),
            "通用白名单会误放行含 sess 的匿名字段"
        );
        assert!(!netease_logged_in_cookie(anonymous), "专属判据必须拒绝它");
    }

    #[test]
    fn supplemental_urls_cover_netease_passport_domain() {
        // 业务入口在 mp.163.com，票据可能落在 reg.163.com 或 passport.163.com。
        let extra: Vec<String> = supplemental_cookie_urls("netease")
            .iter()
            .map(|u| u.as_str().to_string())
            .collect();
        assert!(extra.contains(&"https://reg.163.com/".to_string()));
        assert!(extra.contains(&"https://passport.163.com/".to_string()));
        assert!(extra.contains(&"https://mp.163.com/".to_string()));

        let merged = with_supplemental_urls(
            "netease",
            vec![Url::parse("https://mp.163.com/").unwrap()],
        );
        // 去重按完整 URL 字符串，业务域 `/` 与补充域 `/` 完全相同，会被合并。
        assert_eq!(merged.len(), 3, "登录入口 + reg.163.com + passport.163.com");
        let unique: std::collections::HashSet<&str> =
            merged.iter().map(|u| u.as_str()).collect();
        assert_eq!(unique.len(), merged.len(), "补充地址不得重复");
    }

    #[test]
    fn cookie_key_logging_covers_platforms_with_unstable_key_names() {
        // 这些平台的鉴权键名需靠真机日志校正，必须逐轮打印键名清单。
        assert_eq!(cookie_key_log_label("aliyun"), Some("阿里云"));
        assert_eq!(cookie_key_log_label("oschina"), Some("开源中国"));
        assert_eq!(cookie_key_log_label("baijiahao"), Some("百家号"));
        assert_eq!(cookie_key_log_label("netease"), Some("网易号"));
        // 判据已确定的平台不必刷屏。
        assert_eq!(cookie_key_log_label("51cto"), None);
        assert_eq!(cookie_key_log_label("juejin"), None);

        // 键名清单必须排序去空，且绝不能把值带出来。
        let cookie = "b=2; a=1; =; c=3";
        assert_eq!(cookie_key_names(cookie), vec!["a", "b", "c"]);
    }

    #[test]
    fn has_really_logged_in_routes_51cto_to_dedicated_check() {
        // PHPSESSID 这个名字会命中通用白名单，但 51CTO 必须走专属判定。
        let anonymous = "PHPSESSID=anon; pub_auth_profile=; pub_cookietime=0";
        assert!(!has_really_logged_in("51cto", anonymous));
        assert!(auth_cookie_key(anonymous).is_some());
        assert!(has_really_logged_in("51cto", "PHPSESSID=x; pub_auth_profile=uid_1"));
        // 其他平台不受影响。
        assert!(has_really_logged_in("juejin", "sessionid=abc"));
    }

    #[test]
    fn cookie_with_aliyun_ticket_is_logged_in() {
        // 登录成功后统一登录域下发的 SSO 票据（HttpOnly，页面内读不到）。
        assert!(has_really_logged_in(
            "aliyun",
            "aliyun_country=CN; c_csrf=tok; login_aliyunid_ticket=eyJhbGciOi.payload.sig"
        ));
        // 只有账号标识也算。
        assert!(has_really_logged_in("aliyun", "login_aliyunid=1234567; c_csrf=tok"));
        // 会话键名兜底。
        assert!(has_really_logged_in("aliyun", "aliyun_session=abc"));
        // 键名大小写不敏感。
        assert!(aliyun_logged_in_cookie("LOGIN_ALIYUNID_TICKET=T"));
    }

    #[test]
    fn aliyun_anonymous_cookies_are_not_logged_in() {
        // 匿名访客的真实形态：只有埋点 / 风控 / CSRF 字段，票据缺失或为空。
        let anonymous = "cna=abc; cookie2=1; _tb_token_=e1f2; xlly_s=1; tfstk=a;\
             c_csrf=tok; aliyun_country=CN; aliyun_lang=zh;\
             login_aliyunid=; login_aliyunid_ticket=";
        assert!(!aliyun_logged_in_cookie(anonymous));
        assert!(!has_really_logged_in("aliyun", anonymous));
        // 通用白名单反而会放行：`_tb_token_` 含 `token` 子串，而它是匿名 Cookie。
        // 这正是阿里云必须走专属判据、不能只靠 `AUTH_COOKIE_HINTS` 的原因。
        assert_eq!(auth_cookie_key(anonymous).as_deref(), Some("_tb_token_"));

        // `t` / `cookie2` 这类阿里巴巴系通用字段匿名即存在，绝不能单独作为判据，
        // 否则轮询会在窗口打开的第一秒误判并秒关窗口。
        assert!(!aliyun_logged_in_cookie("t=1700000000; cookie2=abc; cna=xyz"));
        // 值为 0 / null 同样不算。
        assert!(!aliyun_logged_in_cookie("login_aliyunid=0"));
        assert!(!aliyun_logged_in_cookie("login_aliyunid_ticket=null"));
    }

    #[test]
    fn supplemental_urls_cover_aliyun_login_and_account_domains() {
        // SSO 票据下发在统一登录域，c_csrf 下发在业务域，两处都必须查。
        let extra: Vec<String> = supplemental_cookie_urls("aliyun")
            .iter()
            .map(|u| u.as_str().to_string())
            .collect();
        assert!(extra.contains(&"https://login.aliyun.com/".to_string()));
        assert!(extra.contains(&"https://account.aliyun.com/".to_string()));
        assert!(extra.contains(&"https://developer.aliyun.com/".to_string()));

        let merged = with_supplemental_urls(
            "aliyun",
            vec![Url::parse("https://account.aliyun.com/login/login.htm").unwrap()],
        );
        // 登录入口 + 三个补充域，且不含重复地址。
        assert_eq!(merged.len(), 4);
        let unique: std::collections::HashSet<&str> =
            merged.iter().map(|u| u.as_str()).collect();
        assert_eq!(unique.len(), merged.len(), "补充地址不得重复");
    }

    #[test]
    fn probe_script_requires_dom_feature_and_auth_cookie() {
        let script = login_probe_script();
        // 探针必须同时依赖 DOM 特征与鉴权证据，缺一不可。
        assert!(script.contains("var AUTH_KEYS"), "必须有一段鉴权字段白名单");
        assert!(script.contains("document.cookie"), "必须读 document.cookie");
        assert!(
            script.contains("var authOk = auth !== 'none'"),
            "必须要求 DOM 命中且存在鉴权证据才上报成功"
        );
        // 登录页必须一票否决：`blog.51cto.com/login` 的整站导航自带「创作中心」
        // 等文字，任何形式的越权通道都会让窗口在扫码前秒关。
        assert!(
            script.contains("var domOk = !login &&"),
            "登录页必须硬性一票否决，不得存在 loginOnly 越权通道"
        );
        assert!(
            !script.contains("loginOnly"),
            "不得保留 loginOnly 越权通道"
        );
        // 弱词不得进白名单，否则 login_from / BIDUPSID 会被当成鉴权字段。
        assert!(!script.contains("'login', 'sso'"), "弱词 login/sso 不得进入白名单");
        // PHPSESSID 不能只看名字：51CTO 匿名访客同样有。
        assert!(script.contains("name === 'phpsessid'"), "必须跳过 PHPSESSID");
        assert!(
            script.contains("fiftyOneCtoLoggedIn"),
            "51CTO 必须走专属的值判定"
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