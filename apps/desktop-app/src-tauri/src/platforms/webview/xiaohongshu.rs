//! 小红书图文通道适配器（Pinia store 灌数据 → 默认存草稿 / 勾选后自动发布）。
//!
//! 放弃 DOM 填表：小红书的标题框、正文框、上传按钮都是自定义标签
//! （`xhs-publish-btn` 等），placeholder 文案会变，contenteditable 还可能隐藏
//! 在 Shadow DOM 里——靠 DOM selector 等 20 秒大概率超时。
//!
//! 正确的方式是：
//! 1. 等 Webpack 运行时就绪（比 DOM 先加载）
//! 2. 从带 `__vue_app__` 的节点里找 `$pinia._s`（**是 Map，用 `.get()`**）
//! 3. 直接写 `draft.title` / `draft.desc` / `draft.imgList` / `publisher.publishType`
//! 4. imgList 塞带 `url` / `status` / `fileMetadata` 的数组，小红书前端自己会触发
//!    图片下载、签名 upload API、塞进发布 payload
//! 5. 未勾选「覆盖已发布」：先按标题在 IndexedDB 的 `draft-database-v1` /
//!    `image-draft` 里查同名草稿，命中就复用其 `draftId`（该表以 `draftId`
//!    作主键，复用即覆盖，草稿箱里不会越存越多）；随后拦截 `/draft` 或
//!    `/save` 类请求确认草稿落盘，并穿透 Shadow DOM 点击「暂存离开」，成功后关窗
//! 6. 勾选「覆盖已发布」：拦截 `POST /web_api/sns/v2/note` 的响应拿 articleId，
//!    并穿透 Shadow DOM 点击「发布」，成功后关窗
//! 7. 全程 `document.title` 回传状态，Rust 侧轮询后按 `publish` 参数决定文案
//!
//! 注意：小红书网页端草稿写的是当前 WebView 的 IndexedDB，不跨浏览器同步，
//! 所以草稿模式的返回文案会明确提示用户「多端可见请直接发布」。草稿复用也只在
//! 同一个 WebView 内生效——换机器或清缓存后同名草稿查不到，会正常新建。

use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tauri::{AppHandle, Manager};
use tokio::time::{sleep, Instant};

use super::{channel_window_label, open_channel_window};
use crate::platforms::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const PLATFORM: &str = "xiaohongshu";

pub const LOGIN_URL: &str = "https://creator.xiaohongshu.com/";
pub const ARTICLE_URL: &str =
    "https://creator.xiaohongshu.com/publish/publish?from=menu&target=image";

pub const STATUS_PREFIX: &str = "OPENPUB: ";

const SCRIPT_TIMEOUT_MS: u64 = 150_000;
const POLL_INTERVAL_MS: u64 = 400;
const WINDOW_WAIT_MS: u64 = 3_000;
const LOGIN_CHECK_TIMEOUT_MS: u64 = 300_000;
const LOGIN_CHECK_INTERVAL_MS: u64 = 500;

pub fn is_login_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("/login")
}

pub fn is_logged_in_url(url: &str) -> bool {
    if is_login_url(url) {
        return false;
    }
    let Ok(parsed) = tauri::Url::parse(url) else {
        return false;
    };
    if parsed.host_str() != Some("creator.xiaohongshu.com") {
        return false;
    }
    let path = parsed.path();
    path.starts_with("/new/") || path.starts_with("/publish/")
}

async fn ensure_login_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let label = channel_window_label(PLATFORM);

    if let Some(existing) = app.get_webview_window(&label) {
        let url = existing.url().map(|u| u.to_string()).unwrap_or_default();
        if is_logged_in_url(&url) && url.contains("/publish/publish") {
            let _ = existing.set_size(tauri::PhysicalSize::new(1280, 800));
            let _ = existing.set_focus();
            return Ok(existing);
        }
        let _ = existing.close();
    }

    let window = open_channel_window(app, PLATFORM, ARTICLE_URL)?;
    let _ = window.set_size(tauri::PhysicalSize::new(1280, 800));
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();

    let deadline = Instant::now() + Duration::from_millis(LOGIN_CHECK_TIMEOUT_MS);
    while Instant::now() < deadline {
        let url = window.url().map(|u| u.to_string()).unwrap_or_default();
        if url.contains("/publish/publish") && !url.contains("/login") {
            sleep(Duration::from_millis(WINDOW_WAIT_MS)).await;
            return Ok(window);
        }
        if is_login_url(&url) {
            sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
            continue;
        }
        if !url.is_empty() && !url.contains("xiaohongshu") {
            sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
            continue;
        }
        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }

    Err("等待小红书登录超时".to_string())
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
                return Err("小红书通道窗口关闭超时，请稍后重试".to_string());
            }
            sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }
    Ok(())
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
    let deadline = Instant::now() + Duration::from_millis(LOGIN_CHECK_TIMEOUT_MS);
    let mut saw_login_page = false;

    loop {
        if Instant::now() >= deadline {
            crate::login::emit_failed(
                app,
                PLATFORM,
                "小红书登录超时，请重新点击「打开平台页面」扫码",
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
            let url = url.to_string();
            if is_login_url(&url) {
                saw_login_page = true;
            } else if saw_login_page || is_logged_in_url(&url) {
                crate::login::emit_success(app, PLATFORM);
                let _ = window.close();
                return;
            }
        }

        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }
}

fn parse_status(title: &str) -> (String, String) {
    if let Some(rest) = title.strip_prefix(STATUS_PREFIX) {
        if let Some(colon_idx) = rest.find(':') {
            let kind = rest[..colon_idx].trim().to_string();
            let payload = rest[colon_idx + 1..].trim().to_string();
            return (kind, payload);
        }
        return (rest.trim().to_string(), String::new());
    }
    ("UNKNOWN".to_string(), title.to_string())
}

const PUBLISH_SCRIPT_TEMPLATE: &str = r##"(function () {
  var PREFIX = __PREFIX__;
  var P = __PARAMS__;

  var STEP = "INIT";
  var LAST_STEP_AT = Date.now();
  var FAILED = false;
  var TERMINAL = false;
  var WATCHDOG_MS = 60000;

  function sleep(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }

  function setStatus(step) {
    if (FAILED || TERMINAL) return;
    STEP = step;
    LAST_STEP_AT = Date.now();
    document.title = PREFIX + "STEP:" + step;
  }

  function fail(msg) {
    if (FAILED || TERMINAL) return;
    FAILED = true;
    TERMINAL = true;
    document.title = PREFIX + "FAIL:[" + STEP + "] " + String(msg || "未知错误");
  }

  function done(payload) {
    if (FAILED || TERMINAL) return;
    TERMINAL = true;
    document.title = PREFIX + "DONE:" + JSON.stringify(payload);
  }

  window.onerror = function (msg, src, line, col, err) {
    fail("脚本异常: " + ((err && err.message) || msg) + " @" + (src || "?") + ":" + line + ":" + col);
    return false;
  };
  window.addEventListener("unhandledrejection", function (e) {
    var r = e && e.reason;
    fail("未捕获的 Promise 异常: " + ((r && r.message) || r));
  });
  setInterval(function () {
    if (FAILED || TERMINAL) return;
    if (Date.now() - LAST_STEP_AT > WATCHDOG_MS) {
      fail("步骤超时（停在 " + STEP + "）");
    }
  }, 1000);

  function waitFor(fn, timeoutMs, label) {
    var deadline = Date.now() + timeoutMs;
    return new Promise(function (resolve, reject) {
      var timer = setInterval(function () {
        var v = null;
        try { v = fn(); } catch (e) { v = null; }
        if (v) { clearInterval(timer); resolve(v); return; }
        if (Date.now() >= deadline) {
          clearInterval(timer);
          reject(new Error("等待超时：" + label));
        }
      }, 200);
    });
  }

  function findPiniaStores() {
    var all = document.querySelectorAll('*');
    var maps = [];
    for (var i = 0; i < all.length; i++) {
      var app = all[i].__vue_app__;
      var pinia = app && app.config && app.config.globalProperties && app.config.globalProperties.$pinia;
      var map = pinia && pinia._s;
      if (map && typeof map.has === "function" && typeof map.get === "function") maps.push(map);
    }
    for (var j = 0; j < maps.length; j++) {
      var m = maps[j];
      if (m.has("draft") && m.has("publisher")) {
        var keys = [];
        try { m.forEach(function (v, k) { keys.push(String(k)); }); } catch (e) {}
        return {
          map: m,
          draft: m.get("draft"),
          publisher: m.get("publisher"),
          user: m.get("user"),
          article: m.get("article"),
          storeKeys: keys
        };
      }
    }
    return null;
  }

  var DRAFT_DB_NAME = "draft-database-v1";
  var DRAFT_STORE_NAME = "image-draft";

  function openDraftDatabase(timeoutMs) {
    return new Promise(function (resolve) {
      var settled = false;
      var timer = setTimeout(function () {
        if (settled) return;
        settled = true;
        resolve(null);
      }, timeoutMs);
      function finish(db) {
        if (settled) { try { db.close(); } catch (e) {} return; }
        settled = true;
        clearTimeout(timer);
        resolve(db);
      }
      try {
        var request = indexedDB.open(DRAFT_DB_NAME);
        request.onerror = function () {
          if (settled) return;
          settled = true;
          clearTimeout(timer);
          resolve(null);
        };
        request.onupgradeneeded = function () {
          try { request.transaction.abort(); } catch (e) {}
        };
        request.onsuccess = function () { finish(request.result); };
      } catch (e) {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        resolve(null);
      }
    });
  }

  function readDraftRecords(db) {
    return new Promise(function (resolve) {
      try {
        if (!db.objectStoreNames.contains(DRAFT_STORE_NAME)) {
          resolve([]);
          return;
        }
        var tx = db.transaction([DRAFT_STORE_NAME], "readonly");
        var store = tx.objectStore(DRAFT_STORE_NAME);
        var request = store.getAll();
        request.onsuccess = function () {
          resolve(Array.isArray(request.result) ? request.result : []);
        };
        request.onerror = function () { resolve([]); };
      } catch (e) { resolve([]); }
    });
  }

  function normalizeTitle(text) {
    return String(text || "").replace(/\s+/g, " ").trim();
  }

  function draftTitleOf(record) {
    try {
      var store = record && record.content && record.content.draftStore;
      return normalizeTitle(store && store.title);
    } catch (e) { return ""; }
  }

  async function findExistingDraftId(title) {
    var wanted = normalizeTitle(title);
    if (!wanted) return null;
    var db = await openDraftDatabase(5000);
    if (!db) return null;
    var records = await readDraftRecords(db);
    try { db.close(); } catch (e) {}
    for (var i = 0; i < records.length; i++) {
      var record = records[i];
      if (!record) continue;
      if (draftTitleOf(record) !== wanted) continue;
      var id = record.draftId;
      if (id === undefined || id === null || String(id).trim() === "") continue;
      return String(id);
    }
    return null;
  }

  function applyReusedDraftId(stores, draftId) {
    if (!draftId) return;
    try { stores.publisher.draftId = draftId; } catch (e) {}
    try { stores.draft.draftId = draftId; } catch (e) {}
  }

  function extractImages(html) {
    if (!html) return [];
    var results = [];
    var re = /<img[^>]+src=["']([^"']+)["']/gi;
    var m;
    while ((m = re.exec(html)) !== null) {
      var url = m[1];
      if (url.indexOf("http") === 0 || url.indexOf("data:") === 0) results.push(url);
    }
    return results;
  }

  function generateCover(title) {
    try {
      var canvas = document.createElement("canvas");
      canvas.width = 800;
      canvas.height = 600;
      var ctx = canvas.getContext("2d");
      ctx.fillStyle = "#1a1a1a";
      ctx.fillRect(0, 0, 800, 600);
      ctx.fillStyle = "#ffffff";
      ctx.font = "bold 40px -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      var lines = [];
      var currentLine = "";
      var safeTitle = String(title || "无标题");
      for (var i = 0; i < safeTitle.length; i++) {
        currentLine += safeTitle[i];
        if (ctx.measureText(currentLine).width > 700) {
          lines.push(currentLine);
          currentLine = "";
        }
      }
      if (currentLine) lines.push(currentLine);
      var startY = 300 - (lines.length - 1) * 30;
      for (var j = 0; j < lines.length; j++) {
        ctx.fillText(lines[j], 400, startY + j * 60);
      }
      ctx.fillStyle = "rgba(255,255,255,0.3)";
      ctx.font = "20px sans-serif";
      ctx.fillText("OpenPub 智能生成封面", 400, 550);
      return canvas.toDataURL("image/jpeg", 0.85);
    } catch (e) {
      return null;
    }
  }

  function stripHtml(html) {
    if (!html) return "";
    return String(html).replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").trim();
  }

  function base64ToBlob(dataUrl) {
    try {
      var parts = dataUrl.split(',');
      var mime = (parts[0].match(/:(.*?);/) || [])[1] || "image/jpeg";
      var raw = atob(parts[1]);
      var arr = new Uint8Array(raw.length);
      for (var i = 0; i < raw.length; i++) arr[i] = raw.charCodeAt(i);
      return new Blob([arr], { type: mime });
    } catch (e) { return null; }
  }

  async function fetchImageAsBlob(src) {
    if (src.indexOf("data:") === 0) {
      return base64ToBlob(src);
    }
    try {
      var resp = await fetch(src, { credentials: "include", mode: "cors" });
      if (!resp.ok) throw new Error("HTTP " + resp.status);
      return await resp.blob();
    } catch (e) {
      throw new Error("封面图下载失败（" + src.slice(0, 80) + "）：" + ((e && e.message) || e));
    }
  }

  async function uploadImageToDom(blob, stores) {
    var fileInput = await waitFor(function () {
      var inputs = document.querySelectorAll('input[type="file"]');
      if (inputs.length > 0) {
        console.log("[OpenPub 小红书] 发现 " + inputs.length + " 个 input[type=file]，accept=" +
          Array.prototype.map.call(inputs, function (x) { return x.accept || "(空)"; }).join(" | "));
      }
      for (var i = 0; i < inputs.length; i++) {
        var accept = (inputs[i].accept || "").toLowerCase();
        if (accept.indexOf("image") >= 0 || accept === "*" || accept === "") return inputs[i];
      }
      return inputs.length > 0 ? inputs[0] : null;
    }, 20000, "文件上传控件 <input type=file>");

    var ext = "jpg";
    try { ext = (blob.type || "image/jpeg").split("/")[1] || "jpg"; } catch (e) {}
    var file = new File([blob], "cover." + ext, { type: blob.type || "image/jpeg" });
    var dt = new DataTransfer();
    dt.items.add(file);
    fileInput.files = dt.files;
    console.log("[OpenPub 小红书] 已塞入 File（" + file.name + " / " + file.size + " bytes），触发 change");
    fileInput.dispatchEvent(new Event("change", { bubbles: true }));

    var beforeCount = (stores.draft && stores.draft.imgList) ? stores.draft.imgList.length : 0;
    var uploadOk = false;
    try {
      await waitFor(function () {
        var list = stores.draft && stores.draft.imgList;
        if (!list || list.length === 0) return false;
        if (list.length === beforeCount) return false;
        var allDone = true;
        for (var j = 0; j < list.length; j++) {
          if (list[j].status !== 2) { allDone = false; break; }
        }
        return allDone;
      }, 30000, "imgList 图片上传完成（status 全变 2）");
      uploadOk = true;
    } catch (e) {
      console.log("[OpenPub 小红书] imgList 未在 30 秒内就绪：" + ((e && e.message) || e) +
        "，当前 imgList = " + JSON.stringify(stores.draft && stores.draft.imgList));
    }

    if (!uploadOk) {
      var dropzone = document.querySelector(".upload-drag-area, .drag-area, .upload-area, [class*=\"upload\"]");
      if (dropzone) {
        console.log("[OpenPub 小红书] input 未生效，尝试向拖拽区派发 drop 事件");
        var dropDt = new DataTransfer();
        dropDt.items.add(file);
        ["dragenter", "dragover", "drop"].forEach(function (name) {
          var ev = new DragEvent(name, { bubbles: true, cancelable: true, dataTransfer: dropDt });
          dropzone.dispatchEvent(ev);
        });
        await sleep(2000);
      }
    }
  }

  async function run() {
    setStatus("STEP_1_START");

    if (location.origin !== "https://creator.xiaohongshu.com") {
      throw new Error("当前页面不是小红书创作中心：" + location.origin + location.pathname);
    }

    setStatus("STEP_2_WAIT_RUNTIME");
    var runtimeFound = false;
    for (var i = 0; i < 60; i++) {
      if (window.webpackChunkheifetz || window.webpackChunkugc ||
          window.__webpack_require__) {
        runtimeFound = true;
        break;
      }
      var testPinia = findPiniaStores();
      if (testPinia) { runtimeFound = true; break; }
      await sleep(500);
    }
    if (!runtimeFound) {
      throw new Error("等待小红书页面运行时超时（60 秒内未检测到 webpack 或 Pinia）");
    }

    setStatus("STEP_3_FIND_PINIA");
    var stores = await waitFor(findPiniaStores, 15000, "Pinia stores（draft + publisher）");
    var keys = stores.storeKeys || [];
    if (!stores.draft || !stores.publisher) {
      throw new Error("核心 Store 缺失：draft=" + !!stores.draft + " publisher=" + !!stores.publisher + " keys=[" + keys.join(",") + "]");
    }

    setStatus("STEP_4_PREPARE_DATA");
    var images = extractImages(P.content);
    if (images.length === 0) {
      setStatus("STEP_4_GEN_COVER");
      var fallbackCover = generateCover(P.title || "无标题");
      if (!fallbackCover) {
        throw new Error("自动生成封面失败（Canvas 不可用）");
      }
      images = [fallbackCover];
    }

    try {
      if (stores.draft.imgList && stores.draft.imgList.length > 0) {
        stores.draft.imgList.splice(0, stores.draft.imgList.length);
      }
    } catch (e) {}

    stores.draft.title = String(P.title).substring(0, 20);
    stores.draft.desc = stripHtml(P.content).slice(0, 1000) || P.title;
    stores.publisher.publishType = 1;
    stores.publisher.isArticle = false;

    var DO_PUBLISH = P.publish === true;

    // 草稿模式下先找可复用的旧草稿。优先级：
    // 1. Rust 侧从 OpenPub 同步记录里带下来的 `existingDraftId`——这是最可靠的，
    //    标题改多少次都不影响；2. 本地 IndexedDB 按标题兜底（兼容老数据/换机器）。
    // IndexedDB 的 `image-draft` 以 `draftId` 作主键，复用同一个 draftId 就等价于
    // 「覆盖那条草稿」，不会在草稿箱里越存越多。发布模式不查：已发布的笔记本来
    // 就走另一条接口，与草稿箱无关。
    if (!DO_PUBLISH) {
      setStatus("STEP_4_LOOKUP_DRAFT");
      var reusedDraftId = null;
      var carriedDraftId = P.existingDraftId;
      if (carriedDraftId !== undefined && carriedDraftId !== null) {
        var carriedText = String(carriedDraftId).trim();
        if (carriedText !== "" && carriedText !== "0") reusedDraftId = carriedText;
      }
      if (reusedDraftId) {
        console.log("[OpenPub 小红书] 使用 OpenPub 记录的固定草稿 ID =", reusedDraftId);
      } else {
        reusedDraftId = await findExistingDraftId(stores.draft.title);
        if (reusedDraftId) {
          console.log("[OpenPub 小红书] 命中同名草稿，复用 draftId =", reusedDraftId);
        }
      }
      if (reusedDraftId) {
        applyReusedDraftId(stores, reusedDraftId);
      } else {
        console.log("[OpenPub 小红书] 无历史草稿 ID，将新建草稿");
      }
    }

    setStatus("STEP_5_UPLOAD_IMAGE");
    var coverBlob = await fetchImageAsBlob(images[0]);
    if (!coverBlob) {
      throw new Error("封面图获取 Blob 失败");
    }
    await uploadImageToDom(coverBlob, stores);
    await sleep(1000);
    console.log("[OpenPub 小红书] 图片上传完成，imgList =", stores.draft.imgList.map(function (it) { return it.status + ":" + (it.url || "").slice(0, 60); }));

    setStatus(DO_PUBLISH ? "STEP_6_HOOK_PUBLISH" : "STEP_6_HOOK_DRAFT");

    var NOTE_PATH = "/web_api/sns/v2/note";
    var isSuccess = false;

    function isTargetRequest(url, method) {
      if (String(method || "GET").toUpperCase() !== "POST") return false;
      var u = String(url || "");
      if (DO_PUBLISH) {
        return u.indexOf(NOTE_PATH) >= 0 && u.indexOf("/draft") < 0;
      }
      return u.indexOf("/draft") >= 0 || u.indexOf("/save") >= 0;
    }

    function extractDraftId(data) {
      var candidates = [];
      try {
        var inner = data && data.data;
        if (inner) {
          candidates.push(inner.id, inner.draftId, inner.draft_id, inner.noteId, inner.note_id);
          if (inner.draft && typeof inner.draft === "object") {
            candidates.push(inner.draft.id, inner.draft.draftId);
          }
        }
        candidates.push(data && data.draftId, data && data.draft_id);
      } catch (e) {}
      for (var i = 0; i < candidates.length; i++) {
        var raw = candidates[i];
        if (raw === undefined || raw === null) continue;
        var text = String(raw).trim();
        if (text !== "" && text !== "0") return text;
      }
      // 接口没回 draftId 时退回 Pinia 里页面自己生成的 ID：它就是下次覆盖要用的那张「身份证」。
      try {
        var own = stores.publisher && stores.publisher.draftId;
        if (own !== undefined && own !== null && String(own).trim() !== "") {
          return String(own).trim();
        }
      } catch (e) {}
      try {
        var ownDraft = stores.draft && stores.draft.draftId;
        if (ownDraft !== undefined && ownDraft !== null && String(ownDraft).trim() !== "") {
          return String(ownDraft).trim();
        }
      } catch (e) {}
      return "";
    }

    function captureResponse(body) {
      if (isSuccess || TERMINAL) return;
      try {
        var data = typeof body === "string" ? JSON.parse(body) : body;
        if (!data) return;
        var ok = data.success === true || data.code === 0 || data.code === "0";
        if (!ok) return;

        if (DO_PUBLISH) {
          var nid = data.data && (data.data.id || data.data.note_id || data.data.noteId);
          if (!nid) return;
          var nidStr = String(nid);
          var url = (data.data && typeof data.data.share_link === "string" && data.data.share_link)
            ? data.data.share_link
            : "https://www.xiaohongshu.com/discovery/item/" + nidStr;
          isSuccess = true;
          console.log("[OpenPub 小红书] 发布成功！noteId =", nidStr, "url =", url);
          setStatus("STEP_8_PUBLISH_OK");
          setStatus("STEP_10_PUBLISH_SUCCESS");
          done({ articleId: nidStr, articleUrl: url });
        } else {
          isSuccess = true;
          var savedDraftId = extractDraftId(data) || reusedDraftId || "";
          console.log("[OpenPub 小红书] 草稿保存接口确认成功，draftId =", savedDraftId,
            "reused =", !!reusedDraftId);
          setStatus("STEP_8_DRAFT_OK");
          setStatus("STEP_10_SAVE_SUCCESS");
          done({ articleId: null, articleUrl: null, draftId: savedDraftId, reused: !!reusedDraftId });
        }
      } catch (e) {
        console.log("[OpenPub 小红书] 解析接口响应失败：" + ((e && e.message) || e));
      }
    }

    function finishDraftByToast() {
      if (isSuccess || TERMINAL || DO_PUBLISH) return false;
      var bodyText = (document.body && document.body.innerText) || "";
      if (bodyText.indexOf("已保存") >= 0 ||
          bodyText.indexOf("保存成功") >= 0 ||
          bodyText.indexOf("成功保存") >= 0) {
        isSuccess = true;
        console.log("[OpenPub 小红书] 未拦截到草稿接口，但页面出现保存成功提示，按成功处理");
        setStatus("STEP_8_DRAFT_OK");
        setStatus("STEP_10_SAVE_SUCCESS");
        done({ articleId: null, articleUrl: null, draftId: reusedDraftId || extractDraftId(null), reused: !!reusedDraftId });
        return true;
      }
      return false;
    }

    var origFetchPub = window.fetch;
    if (origFetchPub) {
      window.fetch = function (input, init) {
        var url = typeof input === "string" ? input : ((input && input.url) || "");
        var method = String((init && init.method) || (input && input.method) || "GET").toUpperCase();
        var p = origFetchPub.apply(this, arguments);
        if (isTargetRequest(url, method)) {
          p.then(function (resp) {
            if (!resp || !resp.ok) return;
            try { resp.clone().json().then(captureResponse).catch(function () {}); } catch (e) {}
          }).catch(function () {});
        }
        return p;
      };
    }

    var origXHROpenPub = XMLHttpRequest.prototype.open;
    var origXHRSendPub = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method, url) {
      this._openpubPubMethod = method;
      this._openpubPubUrl = String(url);
      return origXHROpenPub.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function () {
      var self = this;
      if (isTargetRequest(self._openpubPubUrl, self._openpubPubMethod)) {
        self.addEventListener("loadend", function () {
          if (self.status < 200 || self.status >= 300) return;
          try { var txt = self.responseText; if (txt) captureResponse(txt); } catch (e) {}
        });
      }
      return origXHRSendPub.apply(this, arguments);
    };

    setStatus(DO_PUBLISH ? "STEP_7_TRIGGER_PUBLISH" : "STEP_7_SAVE_DRAFT");

    var publishBtn = null;
    for (var wi = 0; wi < 20; wi++) {
      publishBtn = document.querySelector("xhs-publish-btn");
      if (publishBtn && publishBtn.getAttribute("is-publish") === "true") break;
      LAST_STEP_AT = Date.now();
      await sleep(1000);
    }
    if (!publishBtn) {
      throw new Error("未找到 xhs-publish-btn 组件（发布组件未渲染）");
    }

    var compInfo = "is-publish=" + publishBtn.getAttribute("is-publish") +
      " save-text=" + publishBtn.getAttribute("save-text") +
      " save-disabled=" + publishBtn.getAttribute("save-disabled");
    console.log("[OpenPub 小红书] 找到发布组件，属性：" + compInfo);

    var targetText = DO_PUBLISH ? "发布" : "暂存离开";
    var clicked = false;

    if (publishBtn.shadowRoot) {
      var shadowBtns = publishBtn.shadowRoot.querySelectorAll("button");
      console.log("[OpenPub 小红书] Shadow DOM 内按钮数 = " + shadowBtns.length +
        "，文字=" + Array.prototype.map.call(shadowBtns, function (b) { return (b.textContent || "").trim(); }).join("|"));
      for (var si = 0; si < shadowBtns.length; si++) {
        var st = (shadowBtns[si].textContent || "").trim();
        var hit = DO_PUBLISH
          ? (st === "发布" || st === "立即发布")
          : (st === "暂存离开" || st.indexOf("暂存") >= 0);
        if (!hit) continue;
        shadowBtns[si].dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
        shadowBtns[si].dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        shadowBtns[si].dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
        shadowBtns[si].click();
        clicked = true;
        console.log("[OpenPub 小红书] 已在 Shadow DOM 内点击：「" + st + "」");
        break;
      }
    } else {
      console.log("[OpenPub 小红书] xhs-publish-btn 没有开放 shadowRoot（closed mode）");
    }

    if (!clicked) {
      console.log("[OpenPub 小红书] Shadow DOM 未命中【" + targetText + "】，改用事件派发");
      if (DO_PUBLISH) {
        publishBtn.dispatchEvent(new CustomEvent("publish", { bubbles: true, composed: true }));
        try { publishBtn.click(); } catch (e) {}
      } else {
        publishBtn.dispatchEvent(new CustomEvent("save", { bubbles: true, composed: true }));
        publishBtn.dispatchEvent(new CustomEvent("save-draft", { bubbles: true, composed: true }));
      }
    }

    setStatus("STEP_9_WAIT_RESPONSE");
    var waitDeadline = Date.now() + 30000;
    while (Date.now() < waitDeadline) {
      if (isSuccess || TERMINAL) break;
      LAST_STEP_AT = Date.now();
      if (!DO_PUBLISH && finishDraftByToast()) break;
      await sleep(1000);
    }

    if (TERMINAL) return;

    throw new Error(DO_PUBLISH
      ? "等待发布接口响应超时（30 秒内未拦截到 POST " + NOTE_PATH + " 成功响应，可能弹了滑块验证）"
      : "等待草稿保存响应超时（30 秒内既未拦截到草稿接口，也未出现保存成功提示）");
  }

  run().catch(function (err) {
    fail((err && err.message) || err);
  });
})();"##;

fn build_publish_script(params_json: &str) -> String {
    let prefix = serde_json::to_string(STATUS_PREFIX)
        .unwrap_or_else(|_| "\"OPENPUB: \"".to_string());
    PUBLISH_SCRIPT_TEMPLATE
        .replace("__PREFIX__", &prefix)
        .replace("__PARAMS__", params_json)
}

struct ScriptOutcome {
    article_id: Option<String>,
    article_url: Option<String>,
    draft_id: Option<String>,
    reused_draft: bool,
}

async fn run_publish_phase(
    window: &tauri::WebviewWindow,
    params_json: &str,
) -> Result<ScriptOutcome, String> {
    let _ = window.set_title(&format!("{}STEP:RESTART", STATUS_PREFIX));
    sleep(Duration::from_millis(150)).await;
    let mut last_title = window.title().unwrap_or_default();

    let script = build_publish_script(params_json);
    if let Err(e) = window.eval(&script) {
        return Err(format!("脚本注入失败：{}", e));
    }

    let deadline = Instant::now() + Duration::from_millis(SCRIPT_TIMEOUT_MS);

    while Instant::now() < deadline {
        let title = window.title().unwrap_or_default();

        if title != last_title {
            last_title = title.clone();
            if title.starts_with(STATUS_PREFIX) {
                let (kind, payload) = parse_status(&title);
                match kind.as_str() {
                    "DONE" => {
                        let parsed: serde_json::Value = serde_json::from_str(&payload)
                            .map_err(|_| "结果 JSON 解析失败".to_string())?;
                        return Ok(ScriptOutcome {
                            article_id: parsed
                                .get("articleId")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            article_url: parsed
                                .get("articleUrl")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            draft_id: parsed
                                .get("draftId")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            reused_draft: parsed
                                .get("reused")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        });
                    }
                    "FAIL" => return Err(payload.to_string()),
                    "STEP" => eprintln!("[xiaohongshu] 进度 {}", payload),
                    other => eprintln!("[xiaohongshu] 未知状态 {}（{}）", other, payload),
                }
            }
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    Err("小红书发布超时".to_string())
}

/// 小红书图文笔记的正文是纯文本描述，Markdown 符号会原样显示在笔记里，
/// 因此在传参前先剥掉标题井号、列表符号、强调符号、代码反引号与链接语法。
fn markdown_patterns() -> &'static [Regex; 5] {
    static PATTERNS: OnceLock<[Regex; 5]> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            // 图片语法整体丢弃（图片另由 imgList 承载）
            Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("图片正则非法"),
            // 普通链接保留链接文字
            Regex::new(r"\[([^\]]*)\]\([^)]*\)").expect("链接正则非法"),
            // 行首标题井号、引用符号、列表符号
            Regex::new(r"(?m)^\s{0,3}(?:#{1,6}\s*|>\s*|[-*+]\s+|\d+\.\s+)").expect("前缀正则非法"),
            // 行内强调与代码符号：三星 / 双星 / 单星（斜体）、双下划线、删除线、反引号
            Regex::new(r"(?:\*\*\*|\*\*|\*|__|~~|`)").expect("强调正则非法"),
            // 残余 HTML 标签（正文可能混入原始 HTML）
            Regex::new(r"<[^>]*>").expect("标签正则非法"),
        ]
    })
}

fn strip_markdown(input: &str) -> String {
    let patterns = markdown_patterns();
    let mut text = patterns[0].replace_all(input, "").to_string();
    text = patterns[1].replace_all(&text, "$1").to_string();
    text = patterns[2].replace_all(&text, "").to_string();
    text = patterns[3].replace_all(&text, "").to_string();
    text = patterns[4].replace_all(&text, "").to_string();

    // 连续空行压成一个，避免描述里出现大片留白
    let mut collapsed: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty()
            && collapsed.last().map(|l: &String| l.trim().is_empty()) == Some(true)
        {
            continue;
        }
        collapsed.push(line.to_string());
    }
    collapsed.join("\n").trim().to_string()
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
    if request.content.trim().is_empty() {
        return delay_return(start, PublishResult::failure(
            FailureClass::Unknown, "正文不能为空".to_string(), operation, false,
        ));
    }

    let window = match ensure_login_window(app).await {
        Ok(w) => w,
        Err(e) => {
            return delay_return(start, PublishResult::failure(
                FailureClass::AuthInvalid,
                format!("小红书登录失败：{}", e),
                operation, false,
            ));
        }
    };

    let params = serde_json::json!({
        "title": request.title.trim(),
        "content": strip_markdown(&request.content),
        "type": "image",
        "publish": request.publish_status,
        "existing_published": request.existing_published,
        "existingDraftId": request.existing_draft_id.clone().unwrap_or_default()
    });

    let result = run_publish_phase(&window, &params.to_string()).await;
    let outcome = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = window.close();
            return delay_return(start, PublishResult::failure(
                FailureClass::PlatformError,
                format!("小红书{}失败：{}", if request.publish_status { "发布" } else { "存草稿" }, e),
                operation, false,
            ));
        }
    };

    let _ = window.close();

    if request.publish_status {
        delay_return(start, PublishResult::success(
            outcome.article_id.unwrap_or_default(),
            outcome.article_url.unwrap_or_else(|| ARTICLE_URL.to_string()),
            "小红书图文已发布到云端".to_string(),
            operation,
            true,
        ))
    } else {
        let message = if outcome.reused_draft {
            "小红书同名草稿已覆盖更新（未新增重复草稿）。注意：小红书网页端草稿不跨浏览器同步，如需多端可见请勾选「覆盖已发布的文章」后直接发布。"
        } else {
            "小红书草稿已保存至本地！请注意：小红书网页端草稿不跨浏览器同步，如需多端可见请勾选「覆盖已发布的文章」后直接发布。"
        };
        let mut result = PublishResult::success(
            String::new(),
            String::new(),
            message.to_string(),
            operation,
            false,
        );
        // 把草稿 ID 交给上层存进 OpenPub 的同步记录，下次同步直接拿它覆盖同一条草稿，
        // 不再依赖标题匹配——标题怎么改都能命中。
        result.draft_id = outcome.draft_id.filter(|id| !id.trim().is_empty());
        delay_return(start, result)
    }
}

fn delay_return(start: Instant, result: PublishResult) -> PublishResult {
    let min_latency = Duration::from_millis(600);
    let elapsed = start.elapsed();
    if elapsed < min_latency {
        std::thread::sleep(min_latency - elapsed);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_detection() {
        assert!(is_login_url("https://creator.xiaohongshu.com/login"));
        assert!(!is_login_url("https://creator.xiaohongshu.com/publish/publish"));
        assert!(!is_login_url("https://creator.xiaohongshu.com/"));
    }

    #[test]
    fn logged_in_url_detection() {
        assert!(is_logged_in_url(
            "https://creator.xiaohongshu.com/publish/publish?target=image"
        ));
        assert!(is_logged_in_url("https://creator.xiaohongshu.com/new/home"));
        assert!(!is_logged_in_url("https://creator.xiaohongshu.com/login"));
        assert!(!is_logged_in_url("https://creator.xiaohongshu.com/"));
    }

    #[test]
    fn article_url_points_to_image_mode() {
        assert!(ARTICLE_URL.contains("target=image"));
        assert!(!ARTICLE_URL.contains("target=article"));
    }

    #[test]
    fn login_channel_ignores_bare_root_and_accepts_home() {
        assert!(!is_login_url("https://creator.xiaohongshu.com/"));
        assert!(!is_logged_in_url("https://creator.xiaohongshu.com/"));
        assert!(is_login_url(
            "https://creator.xiaohongshu.com/login?redirectPath=%2Fnew%2Fhome"
        ));
        assert!(is_logged_in_url("https://creator.xiaohongshu.com/new/home"));
    }

    #[test]
    fn status_prefix_parsing() {
        let (kind, payload) = parse_status("OPENPUB: DONE:{\"articleId\":\"abc123\"}");
        assert_eq!(kind, "DONE");
        assert_eq!(payload, "{\"articleId\":\"abc123\"}");

        let (k2, p2) = parse_status("OPENPUB: FAIL:stores missing");
        assert_eq!(k2, "FAIL");
        assert_eq!(p2, "stores missing");

        let (k3, p3) = parse_status("other title");
        assert_eq!(k3, "UNKNOWN");
        assert_eq!(p3, "other title");
    }

    #[test]
    fn progress_status_is_parsed_as_step() {
        let (kind, payload) = parse_status("OPENPUB: STEP:STEP_4_PREPARE_DATA");
        assert_eq!(kind, "STEP");
        assert_eq!(payload, "STEP_4_PREPARE_DATA");

        let (k2, p2) = parse_status("OPENPUB: STEP:STEP_8_WAIT_RESPONSE");
        assert_eq!(k2, "STEP");
        assert_eq!(p2, "STEP_8_WAIT_RESPONSE");
    }

    #[test]
    fn script_waits_for_runtime_before_pinia() {
        let script = build_publish_script("{}");
        assert!(script.contains("STEP_2_WAIT_RUNTIME"));
        assert!(script.contains("webpackChunkheifetz"));
        assert!(script.contains("webpackChunkugc"));
        assert!(script.contains("__webpack_require__"));
        assert!(script.contains("STEP_3_FIND_PINIA"));
    }

    #[test]
    fn script_uses_pinia_map_get() {
        let script = build_publish_script("{}");
        assert!(script.contains("$pinia"));
        assert!(script.contains("pinia._s"));
        assert!(script.contains("typeof map.has === \"function\""));
        assert!(script.contains("m.get(\"draft\")"));
        assert!(script.contains("m.get(\"publisher\")"));
        assert!(!script.contains("Object.keys(m)"));
    }

    #[test]
    fn script_prepares_image_and_publisher_mode() {
        let params = serde_json::json!({
            "title": "test",
            "content": "<p>hello</p><img src=\"https://example.com/cover.png\" />",
            "type": "image",
            "publish": true,
            "existing_published": false
        });
        let script = build_publish_script(&params.to_string());
        assert!(script.contains("STEP_4_PREPARE_DATA"));
        assert!(script.contains("extractImages"));
        assert!(script.contains("draft.title"));
        assert!(script.contains("draft.desc"));
        assert!(script.contains("publisher.publishType = 1"));
        assert!(script.contains("publisher.isArticle = false"));
        assert!(script.contains("STEP_5_UPLOAD_IMAGE"));
        assert!(script.contains("fetchImageAsBlob"));
        assert!(script.contains("uploadImageToDom"));
        assert!(script.contains("DRAFT_DB_NAME"));
        assert!(script.contains("findExistingDraftId"));
        assert!(script.contains("applyReusedDraftId"));
        assert!(script.contains("STEP_4_LOOKUP_DRAFT"));
    }

    #[test]
    fn script_branches_draft_and_publish_by_param() {
        let script = build_publish_script("{}");
        assert!(script.contains("var DO_PUBLISH = P.publish === true;"));
        assert!(script.contains("STEP_6_HOOK_PUBLISH"));
        assert!(script.contains("STEP_6_HOOK_DRAFT"));
        assert!(script.contains("STEP_7_TRIGGER_PUBLISH"));
        assert!(script.contains("STEP_7_SAVE_DRAFT"));
        assert!(script.contains("STEP_8_PUBLISH_OK"));
        assert!(script.contains("STEP_8_DRAFT_OK"));
        assert!(script.contains("STEP_10_PUBLISH_SUCCESS"));
        assert!(script.contains("STEP_10_SAVE_SUCCESS"));
        assert!(script.contains("暂存离开"));
        assert!(script.contains("/web_api/sns/v2/note"));
        assert!(script.contains("u.indexOf(\"/draft\") < 0"));
        assert!(script.contains("done({ articleId: nidStr, articleUrl: url })"));
        assert!(script.contains("draftId: savedDraftId, reused: !!reusedDraftId"));
        assert!(script.contains("if (!DO_PUBLISH)"));
        assert!(!script.contains("STEP_6_RELOAD_TO_SYNC"));
    }

    #[test]
    fn script_has_draft_toast_fallback() {
        let script = build_publish_script("{}");
        assert!(script.contains("function finishDraftByToast"));
        assert!(script.contains("未拦截到草稿接口，但页面出现保存成功提示"));
        assert!(script.contains("if (!DO_PUBLISH && finishDraftByToast()) break;"));
    }

    #[test]
    fn script_has_error_capture_and_all_steps() {
        let script = build_publish_script("{}");
        assert!(script.contains("window.onerror"));
        assert!(script.contains("unhandledrejection"));
        assert!(script.contains("FAIL:["));
        assert!(script.contains("function setStatus"));
        assert!(script.contains("STEP_1_START"));
        assert!(script.contains("STEP_2_WAIT_RUNTIME"));
        assert!(script.contains("STEP_3_FIND_PINIA"));
        assert!(script.contains("STEP_4_PREPARE_DATA"));
        assert!(script.contains("STEP_5_UPLOAD_IMAGE"));
        assert!(script.contains("STEP_6_HOOK_PUBLISH"));
        assert!(script.contains("STEP_7_TRIGGER_PUBLISH"));
        assert!(script.contains("STEP_9_WAIT_RESPONSE"));
        assert!(script.contains("STEP_10_PUBLISH_SUCCESS"));
    }

    #[test]
    fn script_no_dom_dependency_or_pinia_article_mode() {
        let script = build_publish_script("{}");
        assert!(!script.contains("placeholder*=\"标题\""));
        assert!(!script.contains("placeholder*=\"正文\""));
        assert!(!script.contains("setNativeValue"));
        assert!(script.contains("DataTransfer"));
        assert!(script.contains("input[type=\"file\"]"));
        assert!(!script.contains("target=article"));
        assert!(!script.contains("publishType = 3"));
        assert!(!script.contains("isArticle = true"));
        assert!(!script.contains("新的创作"));
    }

    #[test]
    fn script_inlines_prefix_and_params() {
        let params = serde_json::json!({
            "title": "含引号\"的标题",
            "content": "<img src=\"x.png\"/>",
            "type": "image",
            "publish": true,
            "existing_published": false
        });
        let script = build_publish_script(&params.to_string());
        assert!(!script.contains("__PREFIX__"));
        assert!(!script.contains("__PARAMS__"));
        assert!(script.contains("var PREFIX = \"OPENPUB: \""));
        assert!(script.contains("含引号\\\"的标题"));
    }

    #[test]
    fn script_reuses_existing_draft_only_in_draft_mode() {
        let script = build_publish_script("{}");
        assert!(script.contains("if (!DO_PUBLISH) {"));
        assert!(script.contains("var carriedDraftId = P.existingDraftId;"));
        assert!(script.contains("reusedDraftId = await findExistingDraftId(stores.draft.title);"));
        assert!(script.contains("applyReusedDraftId(stores, reusedDraftId);"));
        assert!(script.contains("reused: !!reusedDraftId"));
    }

    #[test]
    fn script_carries_openpub_draft_id_before_local_lookup() {
        let script = build_publish_script("{}");
        let carried = script
            .find("var carriedDraftId = P.existingDraftId;")
            .expect("脚本应先读 OpenPub 记录的草稿 ID");
        let local = script
            .find("findExistingDraftId(stores.draft.title)")
            .expect("脚本应保留本地兜底查询");
        assert!(
            carried < local,
            "OpenPub 记录的草稿 ID 必须优先于本地标题匹配"
        );
    }

    #[test]
    fn script_extracts_and_returns_draft_id() {
        let script = build_publish_script("{}");
        assert!(script.contains("function extractDraftId(data)"));
        assert!(script.contains("candidates.push(inner.id, inner.draftId"));
        assert!(script.contains("draftId: savedDraftId"));
        assert!(script.contains("var savedDraftId = extractDraftId(data) || reusedDraftId"));
    }

    #[test]
    fn strip_markdown_removes_syntax_and_keeps_text() {
        let source = "## 标题\n\n**加粗**与`代码`\n\n- 列表项\n\n*斜体*\n\n[链接](https://example.com)\n\n![图片](https://example.com/a.png)";
        let plain = strip_markdown(source);
        assert!(!plain.contains('#'), "标题井号应被剥离: {}", plain);
        assert!(!plain.contains('*'), "强调符号应被剥离: {}", plain);
        assert!(!plain.contains('`'), "代码反引号应被剥离: {}", plain);
        assert!(!plain.contains("- 列表项"), "列表符号应被剥离: {}", plain);
        assert!(plain.contains("标题"), "{}", plain);
        assert!(plain.contains("加粗"), "{}", plain);
        assert!(plain.contains("代码"), "{}", plain);
        assert!(plain.contains("列表项"), "{}", plain);
        assert!(plain.contains("斜体"), "{}", plain);
        assert!(plain.contains("链接"), "{}", plain);
        assert!(!plain.contains("https://example.com"), "链接地址应被剥离: {}", plain);
        assert!(!plain.contains("![图片]"), "图片语法应整体丢弃: {}", plain);
    }

    #[test]
    fn strip_markdown_collapses_blank_lines_and_keeps_text_plain() {
        let plain = strip_markdown("第一段\n\n\n\n第二段");
        assert_eq!(plain, "第一段\n\n第二段");
        assert_eq!(strip_markdown(""), "");
    }
}