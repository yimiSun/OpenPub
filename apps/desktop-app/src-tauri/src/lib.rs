use tauri_plugin_store::StoreExt;

mod articles;
mod login;
mod platforms;
mod settings;
mod update;

const COOKIE_STORE_FILE: &str = "credentials.json";

pub(crate) fn settings_timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", seconds)
}

#[cfg(windows)]
fn dpapi_encrypt(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let ok = unsafe {
        CryptProtectData(
            &input_blob,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output_blob,
        )
    };

    if ok == 0 {
        return Err(format!("DPAPI 加密失败 (Win32 Error: {:?})", std::io::Error::last_os_error()));
    }

    let result = unsafe {
        std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize).to_vec()
    };
    unsafe { LocalFree(output_blob.pbData as *mut _) };
    Ok(result)
}

#[cfg(windows)]
fn dpapi_decrypt(ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_ptr() as *mut u8,
    };
    let mut output_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let ok = unsafe {
        CryptUnprotectData(
            &input_blob,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output_blob,
        )
    };

    if ok == 0 {
        return Err(format!(
            "DPAPI 解密失败（可能换过系统用户或凭据已损坏）: {:?}",
            std::io::Error::last_os_error()
        ));
    }

    let result = unsafe {
        std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize).to_vec()
    };
    unsafe { LocalFree(output_blob.pbData as *mut _) };
    Ok(result)
}

#[cfg(not(windows))]
fn dpapi_encrypt(_plaintext: &[u8]) -> Result<Vec<u8>, String> {
    Err("当前平台未实现加密后端，首发版本仅支持 Windows".to_string())
}

#[cfg(not(windows))]
fn dpapi_decrypt(_ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    Err("当前平台未实现解密后端，首发版本仅支持 Windows".to_string())
}

fn encrypt_cookie_to_base64(platform: &str, cookie: &str) -> Result<String, String> {
    if cookie.is_empty() {
        return Err(format!("{} 的 Cookie 不能为空", platform));
    }
    let encrypted = dpapi_encrypt(cookie.as_bytes())?;
    Ok(base64_encode(&encrypted))
}

fn decrypt_cookie_from_base64(platform: &str, stored: &str) -> Result<String, String> {
    let raw = base64_decode(stored)
        .map_err(|e| format!("{} 的 Cookie 密文格式非法: {}", platform, e))?;
    let decrypted = dpapi_decrypt(&raw)
        .map_err(|e| format!("{} 的 Cookie 解密失败: {}", platform, e))?;
    String::from_utf8(decrypted).map_err(|e| format!("{} 的 Cookie 不是合法 UTF-8: {}", platform, e))
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(b2 & 0b0011_1111) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if bytes.len() % 4 != 0 {
        return Err("长度不是 4 的倍数".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut vals = [0u8; 4];
        for (i, b) in chunk.iter().enumerate() {
            vals[i] = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => 0,
                _ => return Err(format!("非法字符: {}", b)),
            };
        }
        out.push((vals[0] << 2) | (vals[1] >> 4));
        if chunk[2] != b'=' {
            out.push((vals[1] << 4) | (vals[2] >> 2));
        }
        if chunk[3] != b'=' {
            out.push((vals[2] << 6) | vals[3]);
        }
    }
    Ok(out)
}

#[tauri::command]
async fn save_platform_config(
    app: tauri::AppHandle,
    platform: String,
    config: serde_json::Value,
) -> Result<(), String> {
    let store = app
        .store("platform_config.json")
        .map_err(|e| format!("打开配置存储失败: {}", e))?;
    let key = format!("{}_config", platform);
    store.set(&key, config);
    store
        .save()
        .map_err(|e| format!("保存配置失败: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn load_platform_config(
    app: tauri::AppHandle,
    platform: String,
) -> Result<serde_json::Value, String> {
    let store = app
        .store("platform_config.json")
        .map_err(|e| format!("打开配置存储失败: {}", e))?;
    let key = format!("{}_config", platform);
    let value = store
        .get(&key)
        .map(|v| v.clone())
        .unwrap_or_else(|| serde_json::Value::Null);
    Ok(value)
}

pub(crate) fn persist_cookie(
    app: &tauri::AppHandle,
    platform: &str,
    cookie: &str,
) -> Result<(), String> {
    crate::login::ensure_login_supported(platform)?;
    if cookie.trim().is_empty() {
        return Err("Cookie 为空，未保存凭据".to_string());
    }
    let ciphertext = encrypt_cookie_to_base64(platform, cookie)?;
    let store = app
        .store(COOKIE_STORE_FILE)
        .map_err(|e| format!("打开凭据存储失败: {}", e))?;
    let key = format!("{}_cookie", platform);
    // 键名只打印 id，不打印 Cookie 内容（密文长度足够定位问题）。
    eprintln!(
        "[login] 写入凭据 platform={} key={} cipher_bytes={}",
        platform,
        key,
        ciphertext.len()
    );
    store.set(&key, serde_json::Value::String(ciphertext));
    store
        .save()
        .map_err(|e| format!("保存凭据存储失败: {}", e))?;
    Ok(())
}

/// 通道登录窗口：登录与发布共用 `channel-{platform}` 窗口实例。
///
/// 只有通道窗口挂载了 `.on_new_window`，微信 / QQ 授权页的 `window.open`
/// 才能正常接管；独立的 `login-{platform}` 窗口没有这层拦截，点击授权按钮
/// 会完全无反应。知乎、简书的扫码登录因此改走这条路径。
#[tauri::command]
async fn open_channel_login_window(
    app: tauri::AppHandle,
    platform: String,
) -> Result<(), String> {
    match platform.trim() {
        "zhihu" => crate::platforms::webview::zhihu::open_login_channel(&app).await,
        "jianshu" => crate::platforms::webview::jianshu::open_login_channel(&app).await,
        "xiaohongshu" => {
            crate::platforms::webview::xiaohongshu::open_login_channel(&app).await
        }
        other => Err(format!("{} 没有独立的通道登录窗口", other)),
    }
}

#[tauri::command]
async fn encrypt_and_store_cookie(
    app: tauri::AppHandle,
    platform: String,
    cookie: String,
) -> Result<(), String> {
    persist_cookie(&app, &platform, &cookie)
}

pub(crate) fn load_stored_cookie(
    app: &tauri::AppHandle,
    platform: &str,
) -> Result<Option<String>, String> {
    let store = app
        .store(COOKIE_STORE_FILE)
        .map_err(|e| format!("打开凭据存储失败: {}", e))?;
    let key = format!("{}_cookie", platform);
    let outcome = match store.get(&key) {
        Some(serde_json::Value::String(stored)) => {
            Ok(Some(decrypt_cookie_from_base64(platform, &stored)?))
        }
        Some(_) => Err(format!("{} 的凭据存储格式异常，可能来自旧版本明文数据", platform)),
        None => Ok(None),
    };
    eprintln!(
        "[login] 读取凭据 platform={} key={} hit={}",
        platform,
        key,
        matches!(&outcome, Ok(Some(_)))
    );
    if let Ok(Some(cookie)) = &outcome {
        let mut names: Vec<&str> = cookie
            .split(';')
            .filter_map(|segment| {
                let name = segment.split('=').next().unwrap_or("").trim();
                if name.is_empty() {
                    None
                } else {
                    Some(name)
                }
            })
            .collect();
        names.sort_unstable();
        eprintln!(
            "[login] 凭据 Cookie 键名 platform={} keys=[{}]",
            platform,
            names.join(",")
        );
    }
    outcome
}

#[tauri::command]
async fn load_cookie(
    app: tauri::AppHandle,
    platform: String,
) -> Result<Option<String>, String> {
    load_stored_cookie(&app, &platform)
}

#[tauri::command]
async fn clear_cookie(app: tauri::AppHandle, platform: String) -> Result<(), String> {
    let store = app
        .store(COOKIE_STORE_FILE)
        .map_err(|e| format!("打开凭据存储失败: {}", e))?;
    let key = format!("{}_cookie", platform);
    store.delete(&key);
    store
        .save()
        .map_err(|e| format!("保存凭据存储失败: {}", e))?;
    Ok(())
}

fn is_usable_post_id(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    !trimmed.is_empty() && trimmed != "0" && trimmed != "(未知)" && trimmed != "未知"
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HostInfo {
    app_version: String,
    os_version: String,
    engine: String,
}

#[cfg(windows)]
fn os_version_string() -> String {
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;

    let mut info: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;

    let status = unsafe { RtlGetVersion(&mut info) };
    if status == 0 {
        format!(
            "Windows {}.{}.{}",
            info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
        )
    } else {
        fallback_os_version()
    }
}

#[cfg(not(windows))]
fn os_version_string() -> String {
    fallback_os_version()
}

fn fallback_os_version() -> String {
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
}

#[tauri::command]
fn get_host_info(app: tauri::AppHandle) -> HostInfo {
    HostInfo {
        app_version: app.package_info().version.to_string(),
        os_version: os_version_string(),
        engine: "Tauri v2 + Rust".to_string(),
    }
}

fn sync_entry_from_result(
    result: &platforms::PublishResult,
    content: &str,
) -> Option<articles::SyncEntry> {
    let draft_id = result
        .draft_id
        .clone()
        .filter(|id| is_usable_post_id(id));
    // 草稿模式可能只回草稿 ID、不回正式文章 ID（小红书就是这样）。此时把草稿 ID
    // 当作远端标识存进 `id`，下一次同步才能定位到同一条草稿并覆盖它。
    let id = if is_usable_post_id(&result.post_id) {
        result.post_id.clone()
    } else {
        draft_id.clone().unwrap_or_default()
    };
    if !is_usable_post_id(&id) {
        return None;
    }
    Some(articles::SyncEntry {
        id,
        published: result.is_published,
        body_hash: articles::body_hash(content),
        draft_id,
    })
}

#[tauri::command]
async fn platform_publish(
    app: tauri::AppHandle,
    platform: String,
    title: String,
    content: String,
    article_id: Option<String>,
    existing_id: Option<String>,
    is_update: Option<bool>,
    publish_status: Option<bool>,
    existing_published: Option<bool>,
    tags: Option<String>,
    existing_draft_id: Option<String>,
) -> Result<platforms::PublishResult, String> {
    let request = platforms::PublishRequest {
        title,
        content: content.clone(),
        existing_id: existing_id.filter(|id| !id.trim().is_empty()),
        is_update: is_update.unwrap_or(false),
        publish_status: publish_status.unwrap_or(false),
        existing_published: existing_published.unwrap_or(false),
        tags: tags.filter(|value| !value.trim().is_empty()),
        existing_draft_id: existing_draft_id.filter(|id| !id.trim().is_empty()),
    };
    let result = platforms::dispatch(&app, &platform, &request).await;

    if let (Some(article_id), Some(entry)) =
        (article_id.as_deref(), sync_entry_from_result(&result, &content))
    {
        articles::save_sync_entry(
            app.clone(),
            article_id.to_string(),
            platform.clone(),
            entry,
        )
        .await?;
    }

    Ok(result)
}

#[tauri::command]
async fn export_sync_log(
    app: tauri::AppHandle,
    content: String,
) -> Result<String, String> {
    use tauri::Manager;
    if content.trim().is_empty() {
        return Err("当前没有可导出的日志".to_string());
    }
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("获取配置目录失败: {}", e))?
        .join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建日志目录失败: {}", e))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("获取系统时间失败: {}", e))?
        .as_secs();
    let filename = format!("sync-log-{}.jsonl", stamp);
    let path = dir.join(filename);
    std::fs::write(&path, content).map_err(|e| format!("写入日志文件失败: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            save_platform_config,
            load_platform_config,
            encrypt_and_store_cookie,
            load_cookie,
            clear_cookie,
            login::open_login_window,
            open_channel_login_window,
            platform_publish,
            export_sync_log,
            get_host_info,
            settings::export_config,
            settings::import_config,
            settings::reset_all_data,
            settings::list_export_files,
            settings::cleanup_invalid_credentials_cmd,
            update::check_update,
            articles::list_articles,
            articles::create_article,
            articles::update_article,
            articles::delete_article,
            articles::load_sync_map,
            articles::save_sync_entry,
            articles::clear_sync_entry
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::{FailureClass, PublishOperation, PublishRequest, PublishResult};

    fn partial_success_result() -> PublishResult {
        PublishResult {
            success: false,
            post_id: "draft-9001".to_string(),
            edit_url: "https://example.com/draft/draft-9001".to_string(),
            message: "草稿已建，发布失败".to_string(),
            failure_class: Some(FailureClass::PlatformError),
            operation: PublishOperation::Create,
            is_published: false,
            draft_id: None,
        }
    }

    #[test]
    fn success_with_real_id_writes_mapping_and_marks_published() {
        let result = PublishResult::success(
            "article-1001",
            "https://example.com/a/1001",
            "发布成功",
            PublishOperation::Create,
            true,
        );
        let entry = sync_entry_from_result(&result, "正文").expect("应写入映射");
        assert_eq!(entry.id, "article-1001");
        assert!(entry.published);
        assert_eq!(entry.body_hash, articles::body_hash("正文"));
    }

    #[test]
    fn success_with_real_id_but_draft_writes_mapping_and_marks_unpublished() {
        let result = PublishResult::success(
            "draft-1002",
            "https://example.com/draft/1002",
            "草稿已保存",
            PublishOperation::Create,
            false,
        );
        let entry = sync_entry_from_result(&result, "正文").expect("应写入映射");
        assert_eq!(entry.id, "draft-1002");
        assert!(!entry.published);
    }

    #[test]
    fn draft_only_result_stores_draft_id_as_sync_id() {
        let mut result = PublishResult::success(
            "",
            "",
            "小红书草稿已保存",
            PublishOperation::Create,
            false,
        );
        result.draft_id = Some("draft-7788".to_string());

        let entry = sync_entry_from_result(&result, "正文")
            .expect("草稿 ID 必须写入映射，否则下次无法覆盖同一条草稿");
        assert_eq!(entry.id, "draft-7788");
        assert_eq!(entry.draft_id.as_deref(), Some("draft-7788"));
        assert!(!entry.published);
    }

    #[test]
    fn result_without_any_id_still_writes_no_mapping() {
        let result = PublishResult::success("", "", "无 ID", PublishOperation::Create, false);
        assert!(sync_entry_from_result(&result, "正文").is_none());
    }

    #[test]
    fn published_result_keeps_draft_id_for_future_draft_overwrite() {
        let mut result = PublishResult::success(
            "note-1234",
            "https://example.com/item/1234",
            "已发布",
            PublishOperation::Update,
            true,
        );
        result.draft_id = Some("draft-5566".to_string());

        let entry = sync_entry_from_result(&result, "正文").expect("应写入映射");
        assert_eq!(entry.id, "note-1234");
        assert_eq!(entry.draft_id.as_deref(), Some("draft-5566"));
        assert!(entry.published);
    }

    #[test]
    fn failure_with_real_id_still_writes_mapping_as_unpublished() {
        let entry = sync_entry_from_result(&partial_success_result(), "正文")
            .expect("两段式中间态应写入映射");
        assert_eq!(entry.id, "draft-9001");
        assert!(!entry.published);
        assert_eq!(entry.body_hash, articles::body_hash("正文"));
    }

    #[test]
    fn success_with_placeholder_id_writes_no_mapping() {
        for placeholder in ["", "0", "未知", "(未知)", "   "] {
            let result = PublishResult::success(
                placeholder,
                "",
                "发布成功",
                PublishOperation::Create,
                true,
            );
            assert!(
                sync_entry_from_result(&result, "正文").is_none(),
                "占位 ID {:?} 不应写入映射",
                placeholder
            );
        }
    }

    #[test]
    fn host_info_payload_exposes_only_three_read_only_fields() {
        let info = HostInfo {
            app_version: "0.1.0".to_string(),
            os_version: "Windows 10.0.19045".to_string(),
            engine: "Tauri v2 + Rust".to_string(),
        };
        let json = serde_json::to_value(&info).unwrap();
        let object = json.as_object().expect("HostInfo 应序列化为对象");
        let mut keys: Vec<&str> = object.keys().map(|key| key.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["appVersion", "engine", "osVersion"]);
        assert_eq!(object["engine"], serde_json::json!("Tauri v2 + Rust"));
    }

    #[test]
    fn os_version_string_never_returns_empty() {
        assert!(!os_version_string().trim().is_empty());
    }

    #[test]
    fn partial_success_mapping_routes_next_sync_to_update_not_create() {
        let entry = sync_entry_from_result(&partial_success_result(), "正文")
            .expect("两段式中间态应写入映射");

        let next_request = PublishRequest {
            title: "标题".to_string(),
            content: "正文".to_string(),
            existing_id: Some(entry.id.clone()),
            is_update: true,
            publish_status: false,
            existing_published: entry.published,
            tags: None,
            existing_draft_id: None,
        };

        let url = platforms::juejin::resolve_url(&next_request, PublishOperation::Update)
            .expect("映射可用时应解析出更新端点");
        assert!(url.contains("article_draft/update"));
        assert!(!url.contains("article_draft/create"));
    }
}