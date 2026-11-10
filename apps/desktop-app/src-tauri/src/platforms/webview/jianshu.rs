//! 简书通道适配器（fetch 建草稿 + 可见窗口 DOM 填表 + 自动保存 + 发布）。
//!
//! 简书 reqwest 纯 HTTP 请求被 WAF 拦截（403 code 9），只有浏览器同源上下文可通过。
//! PUT /author/notes/{id} 虽然返回 200 但服务器静默忽略 content 字段（无 CSRF meta 可提取）。
//!
//! 因此采用可见窗口 DOM 填表方案：
//! 1. 在已登录的隐藏 channel-jianshu 窗口中 fetch 建草稿（此接口不严格校验 CSRF），
//!    拿 noteId + notebookId；
//! 2. 关闭该窗口（cookie 持久化，不丢失）；
//! 3. open_channel_window 新开可见窗口到编辑器深链 /writer#/notebooks/{nb}/notes/{id}；
//! 4. 注入清缓存脚本：localStorage.clear() + sessionStorage.clear() + location.reload()
//!    （清掉 Draft.js 本地缓存，防止覆盖服务器内容）；
//! 5. 等 reload 完成后注入 DOM 填表脚本：等编辑器加载 → execCommand('insertHTML') 填正文
//!    → 等 3s 自动保存（简书前端 API 写回服务器）；
//! 6. 若勾选发布：点"发布文章"按钮 → 等弹窗 → 点"发布"确认 → 等发布完成；
//! 7. 脚本回传结果 → Rust 侧立刻 window.close() 关掉可见窗口。
//!
//! 用户会短暂看到简书编辑器窗口弹出并自动操作，完成后自动关闭。

use std::time::Duration;

use tauri::{AppHandle, Manager};
use tokio::time::{sleep, Instant};

use super::{channel_window_label, is_placeholder_post_id, open_channel_window};
use crate::platforms::shared::markdown;
use crate::platforms::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const PLATFORM: &str = "jianshu";

pub const LOGIN_URL: &str = "https://www.jianshu.com/sign_in";

pub const STATUS_PREFIX: &str = "OPENPUB: ";

const POLL_INTERVAL_MS: u64 = 200;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

const MINIMUM_RUN_LATENCY_MS: u64 = 600;
const DEFAULT_NOTEBOOK: &str = "17526480";
const FETCH_PHASE_TIMEOUT_MS: u64 = 20000;
const DOM_PHASE_TIMEOUT_MS: u64 = 30000;

pub fn is_login_url(url: &str) -> bool {
    let Ok(parsed) = tauri::Url::parse(url) else {
        return false;
    };
    if parsed.host_str() != Some("www.jianshu.com") {
        return false;
    }
    let path = parsed.path();
    path.starts_with("/sign_in") || path.starts_with("/login")
}

pub fn is_logged_in_url(url: &str) -> bool {
    if is_login_url(url) {
        return false;
    }
    let Ok(parsed) = tauri::Url::parse(url) else {
        return false;
    };
    if parsed.host_str() != Some("www.jianshu.com") {
        return false;
    }
    let path = parsed.path();
    path == "/" || path == "/writer" || path == "/writer/"
}

pub fn existing_ref(request: &PublishRequest) -> Option<(Option<String>, String)> {
    let raw = request.existing_id.as_deref()?.trim();
    if raw.is_empty() || is_placeholder_post_id(raw) {
        return None;
    }
    if let Some((notebook, note)) = raw.split_once('/') {
        let notebook = notebook.trim();
        let note = note.trim();
        if !notebook.is_empty() && !note.is_empty() && !is_placeholder_post_id(note) {
            return Some((Some(notebook.to_string()), note.to_string()));
        }
    }
    Some((None, raw.to_string()))
}

pub async fn open_login_channel(app: &AppHandle) -> Result<(), String> {
    close_and_wait(app).await?;
    open_channel_window(app, PLATFORM, LOGIN_URL)?;

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        watch_login(&app_handle).await;
    });
    Ok(())
}

async fn watch_login(app: &AppHandle) {
    let label = channel_window_label(PLATFORM);
    let deadline = Instant::now() + LOGIN_TIMEOUT;

    loop {
        if Instant::now() >= deadline {
            crate::login::emit_failed(
                app,
                PLATFORM,
                "简书登录超时，请重新点击「打开平台页面」扫码",
            );
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.close();
            }
            return;
        }

        let Some(window) = app.get_webview_window(&label) else {
            crate::login::emit_failed(app, PLATFORM, "登录窗口已关闭，未完成登录");
            return;
        };

        if let Ok(url) = window.url() {
            if is_logged_in_url(url.as_str()) {
                crate::login::emit_success(app, PLATFORM);
                let _ = window.hide();
                return;
            }
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

async fn ensure_login_window(app: &AppHandle, label: &str) -> Result<tauri::WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(label) {
        return Ok(w);
    }

    open_channel_window(app, PLATFORM, LOGIN_URL)
        .map_err(|e| format!("无法打开简书登录窗口：{}", e))?;

    let deadline = Instant::now() + LOGIN_TIMEOUT;

    loop {
        if Instant::now() >= deadline {
            let _ = app.get_webview_window(label).and_then(|w| { let _ = w.close(); Some(()) });
            return Err("简书登录超时，请重新扫码".to_string());
        }

        let Some(window) = app.get_webview_window(label) else {
            return Err("登录窗口意外关闭".to_string());
        };

        if let Ok(url) = window.url() {
            if is_logged_in_url(url.as_str()) {
                return Ok(window);
            }
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

pub async fn run(
    app: &AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    let start = Instant::now();

    if request.title.trim().is_empty() {
        return delay_return(start, PublishResult::failure(
            FailureClass::Unknown, "标题不能为空".to_string(), operation, false,
        ));
    }

    let label = channel_window_label(PLATFORM);
    let login_window = match ensure_login_window(app, &label).await {
        Ok(w) => w,
        Err(e) => {
            return delay_return(start, PublishResult::failure(
                FailureClass::AuthInvalid, e, operation, false,
            ));
        }
    };

    let (existing_note, existing_notebook) = match existing_ref(request) {
        Some((nb, note)) => (note, nb.unwrap_or_default()),
        None => (String::new(), String::new()),
    };

    let fetch_params = serde_json::json!({
        "title": request.title.trim().to_string(),
        "existing_note": existing_note,
        "existing_notebook": existing_notebook,
        "default_notebook": DEFAULT_NOTEBOOK,
    });

    let (note_id, notebook_id) = match run_fetch_phase(&login_window, &fetch_params.to_string()).await {
        Ok(v) => v,
        Err(e) => {
            return delay_return(start, PublishResult::failure(
                FailureClass::PlatformError,
                format!("简书准备阶段失败：{}", e),
                operation, false,
            ));
        }
    };

    let _ = login_window.close();
    let close_deadline = Instant::now() + Duration::from_secs(5);
    while app.get_webview_window(&label).is_some() && Instant::now() < close_deadline {
        sleep(Duration::from_millis(200)).await;
    }

    let editor_url = format!(
        "https://www.jianshu.com/writer#/notebooks/{}/notes/{}",
        notebook_id, note_id
    );
    let window = match open_channel_window(app, PLATFORM, &editor_url) {
        Ok(w) => w,
        Err(e) => {
            return delay_return(start, PublishResult::failure(
                FailureClass::PlatformError,
                format!("简书编辑器窗口打开失败：{}", e),
                operation, false,
            ));
        }
    };

    let nav_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < nav_deadline {
        if let Ok(cur_url) = window.url() {
            let s = cur_url.as_str();
            if s.contains(&format!("/notes/{}", note_id)) && s.contains("/writer") {
                break;
            }
        }
        sleep(Duration::from_millis(200)).await;
    }

    sleep(Duration::from_millis(1200)).await;

    let clear_script = format!(
        "(function() {{ localStorage.clear(); sessionStorage.clear(); document.title = '{}CLEARED'; location.reload(); }})();",
        STATUS_PREFIX
    );
    let _ = window.eval(&clear_script);

    let cleared_deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_cleared = false;
    while Instant::now() < cleared_deadline {
        let title = window.title().unwrap_or_default();
        if !saw_cleared && title == format!("{}CLEARED", STATUS_PREFIX) {
            saw_cleared = true;
        }
        if saw_cleared && !title.starts_with(STATUS_PREFIX) {
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }

    sleep(Duration::from_millis(1500)).await;

    let content_html = markdown::to_html_content(&request.content);
    let dom_params = serde_json::json!({
        "title": request.title.trim().to_string(),
        "content": content_html,
        "publish": request.publish_status,
        "existing_published": request.existing_published,
    });

    let dom_result = run_dom_phase(&window, &dom_params.to_string()).await;
    let (published, content_ok, slug, autosave_detected, autosave_url) = match dom_result {
        Ok(v) => v,
        Err(e) => {
            let _ = window.close();
            return delay_return(start, PublishResult::failure(
                FailureClass::PlatformError,
                format!("简书编辑阶段失败：{}", e),
                operation, false,
            ));
        }
    };

    let _ = window.close();

    let article_id = format!("{}/{}", notebook_id, note_id);
    let edit_url = if published && !slug.is_empty() {
        format!("https://www.jianshu.com/p/{}", slug)
    } else {
        format!("https://www.jianshu.com/writer#/notebooks/{}/notes/{}", notebook_id, note_id)
    };

    let mut msg = if published {
        format!("简书已发布（https://www.jianshu.com/p/{}）", slug)
    } else if content_ok {
        format!("简书草稿已保存（https://www.jianshu.com/writer#/notebooks/{}/notes/{}）", notebook_id, note_id)
    } else {
        format!("简书草稿已提交（内容验证未通过）（https://www.jianshu.com/writer#/notebooks/{}/notes/{}）", notebook_id, note_id)
    };
    if !autosave_detected {
        msg.push_str(" ⚠️未检测到自动保存请求");
    } else if !autosave_url.is_empty() {
        msg.push_str(&format!(" 自动保存已触发（{}）", autosave_url));
    }

    delay_return(start, PublishResult::success(
        article_id, edit_url, msg, operation, false,
    ))
}

async fn run_fetch_phase(
    window: &tauri::WebviewWindow,
    params_json: &str,
) -> Result<(String, String), String> {
    let script = build_fetch_create_script(params_json);
    if let Err(e) = window.eval(&script) {
        return Err(format!("脚本注入失败：{}", e));
    }

    let mut last_title = String::new();
    let deadline = Instant::now() + Duration::from_millis(FETCH_PHASE_TIMEOUT_MS);

    while Instant::now() < deadline {
        let title = window.title().unwrap_or_default();
        if let Some(status) = title.strip_prefix(STATUS_PREFIX) {
            if status != last_title.as_str() {
                let (kind, payload) = parse_status(status);
                match kind.as_str() {
                    "DONE" => {
                        let parsed: serde_json::Value = serde_json::from_str(&payload)
                            .map_err(|_| "结果解析失败".to_string())?;
                        let note_id = parsed.get("id").and_then(|v| v.as_str())
                            .unwrap_or("").to_string();
                        let notebook_id = parsed.get("notebook").and_then(|v| v.as_str())
                            .unwrap_or("").to_string();
                        if note_id.is_empty() {
                            return Err("未拿到文章 id".to_string());
                        }
                        return Ok((note_id, notebook_id));
                    }
                    "NEED_LOGIN" => return Err("登录态已失效".to_string()),
                    "FAIL" => return Err(payload.to_string()),
                    _ => return Err(format!("未知状态：{}", kind)),
                }
            }
        }
        last_title = title;
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    Err("建草稿超时".to_string())
}

async fn run_dom_phase(
    window: &tauri::WebviewWindow,
    params_json: &str,
) -> Result<(bool, bool, String, bool, String), String> {
    let script = build_dom_fill_script(params_json);
    if let Err(e) = window.eval(&script) {
        return Err(format!("DOM脚本注入失败：{}", e));
    }

    let mut last_title = String::new();
    let deadline = Instant::now() + Duration::from_millis(DOM_PHASE_TIMEOUT_MS);

    while Instant::now() < deadline {
        let title = window.title().unwrap_or_default();
        if let Some(status) = title.strip_prefix(STATUS_PREFIX) {
            if status != last_title.as_str() {
                let (kind, payload) = parse_status(status);
                match kind.as_str() {
                    "DONE" => {
                        let parsed: serde_json::Value = serde_json::from_str(&payload)
                            .map_err(|_| "DOM结果解析失败".to_string())?;
                        let published = parsed.get("published").and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let content_ok = parsed.get("content_ok").and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let slug = parsed.get("slug").and_then(|v| v.as_str())
                            .unwrap_or("").to_string();
                        let autosave = parsed.get("autosave").and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let autosave_url = parsed.get("autosave_url").and_then(|v| v.as_str())
                            .unwrap_or("").to_string();
                        return Ok((published, content_ok, slug, autosave, autosave_url));
                    }
                    "NEED_LOGIN" => return Err("登录态已失效".to_string()),
                    "FAIL" => return Err(payload.to_string()),
                    _ => return Err(format!("未知状态：{}", kind)),
                }
            }
        }
        last_title = title;
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    Err("编辑器操作超时".to_string())
}

fn parse_status(raw: &str) -> (String, String) {
    let colon = raw.find(':').unwrap_or(raw.len());
    let kind = raw[..colon].trim().to_string();
    let rest = if colon < raw.len() {
        raw[colon + 1..].trim().to_string()
    } else {
        String::new()
    };
    (kind, rest)
}

fn build_fetch_create_script(params_json: &str) -> String {
    let prefix = STATUS_PREFIX;
    format!(
        r#"(function() {{
  var PREFIX = "{prefix}";
  var P = {params_json};
  async function readJson(r) {{
    var t = await r.text(); var b = null; try {{ b = t ? JSON.parse(t) : null; }} catch(e) {{}}
    return {{ ok: r.ok, status: r.status, body: b, text: String(t || '').slice(0, 500) }};
  }}
  async function pickNotebook() {{
    try {{
      var r = await fetch('https://www.jianshu.com/author/notebooks', {{ method: 'GET', credentials: 'include', headers: {{ Accept: 'application/json' }} }});
      if (r.status === 401 || r.status === 403) throw new Error('NEED_LOGIN');
      var j = await readJson(r);
      if (j.ok && j.body && Array.isArray(j.body) && j.body.length > 0) {{
        return String(j.body[0].id);
      }}
    }} catch(e) {{ if (String((e && e.message) || e) === 'NEED_LOGIN') throw e; }}
    return P.default_notebook;
  }}
  async function createNote(nb, title) {{
    var r = await fetch('https://www.jianshu.com/author/notes', {{ method: 'POST', credentials: 'include', headers: {{ Accept: 'application/json', 'Content-Type': 'application/json' }}, body: JSON.stringify({{ at_bottom: false, notebook_id: nb, title: title }}) }});
    if (r.status === 401 || r.status === 403) throw new Error('NEED_LOGIN');
    var j = await readJson(r);
    if (!j.ok || !j.body || !j.body.id) throw new Error('建草稿失败 HTTP ' + j.status + ' ' + j.text);
    return {{ id: String(j.body.id), slug: String(j.body.slug || '') }};
  }}
  async function fetchNoteMeta(noteId) {{
    var r = await fetch('https://www.jianshu.com/author/notes/' + noteId, {{ method: 'GET', credentials: 'include', headers: {{ Accept: 'application/json' }} }});
    if (r.status === 401 || r.status === 403) throw new Error('NEED_LOGIN');
    var j = await readJson(r);
    if (!j.ok || !j.body) throw new Error('获取草稿信息失败');
    var body = j.body;
    if (Array.isArray(body) && body.length > 0) body = body[0];
    var nb = String((body && body.notebook_id) || P.existing_notebook || '');
    var slug = String((body && body.slug) || '');
    return {{ notebook: nb, slug: slug }};
  }}
  async function run() {{
    try {{
      var nb = '', noteId = '', slug = '';
      if (P.existing_note) {{
        noteId = P.existing_note;
        var meta = await fetchNoteMeta(noteId);
        nb = meta.notebook || P.existing_notebook || await pickNotebook();
        slug = meta.slug;
      }} else {{
        nb = P.existing_notebook || await pickNotebook();
        var created = await createNote(nb, P.title);
        noteId = created.id;
        slug = created.slug;
      }}
      document.title = PREFIX + 'DONE:' + JSON.stringify({{ id: noteId, notebook: nb, slug: slug }});
    }} catch(err) {{
      var msg = String((err && err.message) || err);
      if (msg === 'NEED_LOGIN') {{ document.title = PREFIX + 'NEED_LOGIN'; }}
      else {{ document.title = PREFIX + 'FAIL:' + msg; }}
    }}
  }}
  run();
}})();"#,
        prefix = prefix,
        params_json = params_json,
    )
}

fn build_dom_fill_script(params_json: &str) -> String {
    let prefix = STATUS_PREFIX;
    format!(
        r#"(function() {{
  var PREFIX = "{prefix}";
  var P = {params_json};
  var DIAG = {{}};
  var AUTOSAVE = {{ detected: false, url: '', method: '', time: 0 }};
  function sleep(ms) {{ return new Promise(function(r) {{ setTimeout(r, ms); }}); }}
  function qsa(sel) {{ return Array.prototype.slice.call(document.querySelectorAll(sel)); }}
  function isVisible(el) {{
    if (!el) return false;
    if (!el.offsetParent && el.style.position !== 'fixed' && el.style.position !== 'sticky') return false;
    var r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return false;
    var style = window.getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return false;
    return true;
  }}

  (function hookAutosave() {{
    var origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function(method, url) {{
      try {{
        var s = String(url || '');
        if (s.indexOf('/author/notes') >= 0) {{
          AUTOSAVE.detected = true;
          AUTOSAVE.url = s;
          AUTOSAVE.method = String(method || '');
          AUTOSAVE.time = Date.now();
        }}
      }} catch(e) {{}}
      return origOpen.apply(this, arguments);
    }};
    var origFetch = window.fetch;
    if (origFetch) {{
      window.fetch = function(input, init) {{
        try {{
          var s = typeof input === 'string' ? input : (input && input.url) || '';
          if (s.indexOf('/author/notes') >= 0) {{
            AUTOSAVE.detected = true;
            AUTOSAVE.url = s;
            AUTOSAVE.method = (init && init.method) || 'GET';
            AUTOSAVE.time = Date.now();
          }}
        }} catch(e) {{}}
        return origFetch.apply(this, arguments);
      }};
    }}
  }})();

  function findTitleInput() {{
    var el = document.querySelector('input[placeholder*="标题"], input[placeholder*="文章标题"], input[name="title"]');
    if (el) return el;
    var tabIdx = document.querySelector('div[tabindex="-1"] input[type="text"], div[tabindex="-1"] input');
    if (tabIdx && isVisible(tabIdx)) return tabIdx;
    var inputs = qsa('input[type="text"], input:not([type])');
    for (var i = 0; i < inputs.length; i++) {{
      if (isVisible(inputs[i])) return inputs[i];
    }}
    return null;
  }}
  function findBodyEditor() {{
    var el = document.querySelector('.public-DraftEditor-content');
    if (el && isVisible(el)) return el;
    var eds = qsa('[contenteditable="true"][data-contents="true"], [contenteditable="true"][role="textbox"], .public-DraftEditor-content[contenteditable]');
    for (var i = 0; i < eds.length; i++) {{
      if (isVisible(eds[i])) return eds[i];
    }}
    var allCE = qsa('[contenteditable="true"]');
    for (var j = 0; j < allCE.length; j++) {{
      if (isVisible(allCE[j])) return allCE[j];
    }}
    return null;
  }}
  function findPublishBtn() {{
    var sels = ['button.publish', 'button.publish-btn', '.publish-btn', 'a.publish', '[data-action="publish"]'];
    for (var i = 0; i < sels.length; i++) {{
      var el = document.querySelector(sels[i]);
      if (el && isVisible(el)) return el;
    }}
    var all = qsa('button, a, [role="button"]');
    for (var j = 0; j < all.length; j++) {{
      if (!isVisible(all[j])) continue;
      var t = (all[j].innerText || all[j].textContent || '').trim();
      if (t === '发布文章' || t === '立即发布' || t === '发布') return all[j];
    }}
    return null;
  }}
  function findConfirmBtn() {{
    var all = qsa('button, a, [role="button"]');
    for (var i = 0; i < all.length; i++) {{
      if (!isVisible(all[i])) continue;
      var t = (all[i].innerText || all[i].textContent || '').trim();
      if (t === '发布' || t === '确定' || t === '确认发布' || t === '确认') return all[i];
    }}
    return null;
  }}

  async function fillBody(contentEl, html) {{
    contentEl.focus();
    await sleep(150);

    document.execCommand('selectAll', false, null);
    await sleep(50);
    document.execCommand('delete', false, null);
    await sleep(150);

    var needle = html.replace(/<[^>]*>/g, '').slice(0, 30);
    var hasContentBefore = contentEl.innerText && contentEl.innerText.indexOf(needle) >= 0;

    try {{
      var dt = new DataTransfer();
      dt.setData('text/html', html);
      dt.setData('text/plain', html.replace(/<[^>]*>/g, ''));
      var pasteEvt = new ClipboardEvent('paste', {{
        bubbles: true, cancelable: true,
        clipboardData: dt
      }});
      contentEl.dispatchEvent(pasteEvt);
      await sleep(400);
    }} catch(e) {{}}

    var textNow = (contentEl.innerText || contentEl.textContent || '');
    var pasteOk = needle.length > 0 && textNow.indexOf(needle) >= 0;

    if (!pasteOk) {{
      document.execCommand('selectAll', false, null);
      document.execCommand('delete', false, null);
      await sleep(100);
      document.execCommand('insertHTML', false, html);
      contentEl.dispatchEvent(new Event('input', {{ bubbles: true }}));
      await sleep(300);
    }}

    document.execCommand('selectAll', false, null);
    document.execCommand('delete', false, null);
    await sleep(50);
    document.execCommand('insertHTML', false, html);
    contentEl.dispatchEvent(new Event('input', {{ bubbles: true }}));
  }}

  async function waitAutosave(maxWaitMs) {{
    var deadline = Date.now() + maxWaitMs;
    while (Date.now() < deadline) {{
      if (AUTOSAVE.detected) return true;
      await sleep(200);
    }}
    return AUTOSAVE.detected;
  }}

  async function run() {{
    try {{
      var deadline = Date.now() + 10000;
      while (Date.now() < deadline) {{
        DIAG.input_total = qsa('input').length;
        DIAG.input_text_total = qsa('input[type="text"], input:not([type])').length;
        DIAG.input_visible_count = qsa('input[type="text"], input:not([type])').filter(isVisible).length;
        DIAG.editor_total = qsa('.public-DraftEditor-content').length;
        DIAG.ce_total = qsa('[contenteditable="true"]').length;
        DIAG.title_el = findTitleInput();
        DIAG.content_el = findBodyEditor();
        if (DIAG.title_el && DIAG.content_el) break;
        await sleep(200);
      }}

      if (!DIAG.title_el) {{
        var d = DIAG;
        throw new Error('标题输入框未找到. inputs=' + d.input_total + ' textInputs=' + d.input_text_total + ' visible=' + d.input_visible_count + ' url=' + location.href);
      }}
      if (!DIAG.content_el) {{
        var d2 = DIAG;
        throw new Error('编辑器未找到. editors=' + d2.editor_total + ' contenteditable=' + d2.ce_total + ' url=' + location.href);
      }}

      var titleEl = DIAG.title_el;
      var contentEl = DIAG.content_el;

      AUTOSAVE.detected = false;
      AUTOSAVE.url = '';
      AUTOSAVE.method = '';

      var nativeSet = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      nativeSet.call(titleEl, P.title);
      titleEl.dispatchEvent(new Event('input', {{ bubbles: true }}));
      await sleep(200);

      await fillBody(contentEl, P.content);

      var autosaveFired = await waitAutosave(10000);
      if (!autosaveFired) {{
        await sleep(3000);
      }}

      var checkEl = document.querySelector('.public-DraftEditor-content') || contentEl;
      var needle = P.content.replace(/<[^>]*>/g, '').slice(0, 20);
      var contentOk = false;
      if (checkEl) {{
        var txt = (checkEl.innerText || checkEl.textContent || '');
        contentOk = needle.length > 0 && txt.indexOf(needle) >= 0;
      }}

      var published = false, slug = '';
      if (P.publish && !P.existing_published) {{
        var pubBtn = findPublishBtn();
        if (!pubBtn) throw new Error('发布按钮未找到');
        pubBtn.click();
        await sleep(800);

        var confirmBtn = findConfirmBtn();
        if (confirmBtn && confirmBtn !== pubBtn) {{
          confirmBtn.click();
          await sleep(800);
        }}

        await sleep(3000);

        var m = location.href.match(/\/p\/([a-zA-Z0-9]+)/);
        if (m) {{ published = true; slug = m[1]; }}
        else {{
          var metaSlug = document.querySelector('meta[property="article:url"]');
          if (metaSlug) {{
            var sm = (metaSlug.content || '').match(/\/p\/([a-zA-Z0-9]+)/);
            if (sm) {{ published = true; slug = sm[1]; }}
          }}
        }}
      }}

      document.title = PREFIX + 'DONE:' + JSON.stringify({{
        published: published,
        content_ok: contentOk,
        slug: slug,
        autosave: AUTOSAVE.detected,
        autosave_url: AUTOSAVE.url,
        autosave_method: AUTOSAVE.method
      }});
    }} catch(err) {{
      var msg = String((err && err.message) || err);
      if (msg === 'NEED_LOGIN') {{ document.title = PREFIX + 'NEED_LOGIN'; }}
      else {{ document.title = PREFIX + 'FAIL:' + msg; }}
    }}
  }}
  run();
}})();"#,
        prefix = prefix,
        params_json = params_json,
    )
}

fn delay_return(start: Instant, result: PublishResult) -> PublishResult {
    let elapsed = start.elapsed();
    let wait = MINIMUM_RUN_LATENCY_MS.saturating_sub(elapsed.as_millis() as u64);
    if wait > 0 {
        std::thread::sleep(std::time::Duration::from_millis(wait));
    }
    result
}

async fn close_and_wait(app: &AppHandle) -> Result<(), String> {
    let label = channel_window_label(PLATFORM);
    if app.get_webview_window(&label).is_some() {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.get_webview_window(&label).is_some() {
            if Instant::now() >= deadline {
                return Err("简书通道窗口关闭超时，请稍后重试".to_string());
            }
            sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }
    Ok(())
}