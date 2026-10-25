use tauri_plugin_store::StoreExt;

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

#[tauri::command]
async fn encrypt_and_store_cookie(
    app: tauri::AppHandle,
    platform: String,
    cookie: String,
) -> Result<(), String> {
    let store = app
        .store("credentials.json")
        .map_err(|e| format!("打开存储失败: {}", e))?;
    let key = format!("{}_cookie", platform);
    store.set(&key, serde_json::Value::String(cookie));
    store
        .save()
        .map_err(|e| format!("保存存储失败: {}", e))?;
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[derive(serde::Serialize)]
struct PublishResult {
    success: bool,
    post_id: String,
    edit_url: String,
    message: String,
}

async fn publish_cnblogs(
    app: &tauri::AppHandle,
    title: &str,
    content: &str,
) -> Result<PublishResult, String> {
    let store = app
        .store("platform_config.json")
        .map_err(|e| format!("读取配置失败: {}", e))?;

    let config = store
        .get("cnblogs_config")
        .ok_or("博客园未配置，请先在设置中填写凭据")?
        .clone();

    let blog_name = config.get("blogName").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let username = config.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let api_token = config.get("apiToken").and_then(|v| v.as_str()).unwrap_or("").to_string();

    if blog_name.is_empty() || username.is_empty() || api_token.is_empty() {
        return Err("博客园凭据不完整，请先配置博客名、用户名和访问令牌".to_string());
    }

    let endpoint_url = format!("https://rpc.cnblogs.com/metaweblog/{}", blog_name);

    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<methodCall>\
  <methodName>metaWeblog.newPost</methodName>\
  <params>\
    <param><value><string>{blogid}</string></value></param>\
    <param><value><string>{username}</string></value></param>\
    <param><value><string>{token}</string></value></param>\
    <param><value><struct>\
      <member><name>title</name><value><string>{title}</string></value></member>\
      <member><name>description</name><value><string>{content}</string></value></member>\
      <member><name>categories</name><value><array><data>\
        <value><string>OpenPub</string></value>\
      </data></array></value></member>\
    </struct></value></param>\
    <param><value><boolean>0</boolean></value></param>\
  </params>\
</methodCall>",
        blogid = xml_escape(&blog_name),
        username = xml_escape(&username),
        token = xml_escape(&api_token),
        title = xml_escape(title),
        content = xml_escape(content),
    );

    let client = reqwest::Client::new();
    let response = client
        .post(&endpoint_url)
        .header("Content-Type", "text/xml; charset=utf-8")
        .header("User-Agent", "OpenPub/0.1.0")
        .body(bytes::Bytes::from(body))
        .send()
        .await
        .map_err(|e| format!("请求发送失败: {}", e))?;

    let status = response.status();
    let response_text = response.text().await.map_err(|e| format!("读取响应失败: {}", e))?;

    if response_text.contains("<fault>") {
        let fault_start = response_text.find("<fault>").unwrap_or(0);
        let fault_end = response_text.find("</fault>").unwrap_or(response_text.len());
        let fault_section = &response_text[fault_start..fault_end + 8];
        Ok(PublishResult {
            success: false,
            post_id: String::new(),
            edit_url: String::new(),
            message: format!("MetaWeblog 错误 (HTTP {}): {}", status, fault_section),
        })
    } else if status.is_success() {
        let post_id = response_text
            .find("<string>")
            .and_then(|start| {
                let end = response_text[start..].find("</string>")?;
                Some(&response_text[start + 8..start + end])
            })
            .unwrap_or("(未知)")
            .to_string();

        let edit_url = format!("https://i.cnblogs.com/posts/edit;postId={}", post_id);

        Ok(PublishResult {
            success: true,
            post_id: post_id.clone(),
            edit_url,
            message: format!("发布成功! 文章 ID: {}", post_id),
        })
    } else {
        Ok(PublishResult {
            success: false,
            post_id: String::new(),
            edit_url: String::new(),
            message: format!("HTTP {}, 响应: {}", status, &response_text[..response_text.len().min(300)]),
        })
    }
}

async fn publish_juejin(
    app: &tauri::AppHandle,
    title: &str,
    content: &str,
) -> Result<PublishResult, String> {
    let store = app
        .store("platform_config.json")
        .map_err(|e| format!("读取配置失败: {}", e))?;

    let config = store
        .get("juejin_config")
        .ok_or("掘金未配置，请先在设置中填写 Cookie")?
        .clone();

    let cookie = config
        .get("cookie")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if cookie.is_empty() {
        return Err("掘金 Cookie 不能为空，请先配置".to_string());
    }

    let request_body = serde_json::json!({
        "title": title,
        "markdown": content,
        "content": content,
        "category_id": "0",
        "tag_ids": ["6809637777543250"],
        "brief": &content[..content.len().min(80)],
    });

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.juejin.cn/content_api/v1/article_draft/create")
        .header("Content-Type", "application/json")
        .header("Cookie", &cookie)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .header("Referer", "https://juejin.cn/")
        .header("Origin", "https://juejin.cn")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("请求发送失败: {}", e))?;

    let status = response.status();
    let response_text = response.text().await.map_err(|e| format!("读取响应失败: {}", e))?;

    eprintln!("[juejin] 原始响应: {}", &response_text[..response_text.len().min(500)]);

    let response_json: serde_json::Value =
        serde_json::from_str(&response_text).unwrap_or_else(|_| serde_json::json!({}));

    let err_no = response_json
        .get("err_no")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);

    if err_no == 0 {
        let draft_id = response_json
            .get("data")
            .and_then(|d| d.get("draft_id"))
            .map(|v| {
                if v.is_string() {
                    v.as_str().unwrap_or("").to_string()
                } else {
                    v.to_string()
                }
            })
            .unwrap_or_else(|| "(未知)".to_string());

        let edit_url = format!("https://juejin.cn/editor/drafts/{}", draft_id);

        Ok(PublishResult {
            success: true,
            post_id: draft_id.clone(),
            edit_url,
            message: format!("草稿创建成功! Draft ID: {}", draft_id),
        })
    } else {
        let err_msg = response_json
            .get("err_msg")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let detail = if err_msg.is_empty() {
            response_text[..response_text.len().min(300)].to_string()
        } else {
            err_msg
        };
        Ok(PublishResult {
            success: false,
            post_id: String::new(),
            edit_url: String::new(),
            message: format!(
                "掘金 API 错误 (err_no: {}, HTTP {}): {}",
                err_no, status, detail
            ),
        })
    }
}

#[tauri::command]
async fn platform_publish(
    app: tauri::AppHandle,
    platform: String,
    title: String,
    content: String,
) -> Result<PublishResult, String> {
    match platform.as_str() {
        "cnblogs" => publish_cnblogs(&app, &title, &content).await,
        "juejin" => publish_juejin(&app, &title, &content).await,
        "zhihu" => Err("知乎适配器开发中".to_string()),
        "csdn" => Err("CSDN 适配器开发中".to_string()),
        "wechat" => Err("微信公众号适配器开发中".to_string()),
        "segmentfault" => Err("思否适配器开发中".to_string()),
        "tencent" => Err("腾讯云适配器开发中".to_string()),
        "aliyun" => Err("阿里云适配器开发中".to_string()),
        _ => Err(format!("未知平台: {}", platform)),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            save_platform_config,
            load_platform_config,
            encrypt_and_store_cookie,
            platform_publish
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}