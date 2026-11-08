use tauri_plugin_store::StoreExt;

use super::shared::truncate;
use super::{FailureClass, PlatformError, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "cnblogs_config";
const ERROR_SNIPPET_CHARS: usize = 300;

fn truncate_chars(input: &str, limit: usize) -> String {
    truncate::truncate_chars(input, limit)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn publish_boolean(publish_status: bool) -> &'static str {
    if publish_status {
        "1"
    } else {
        "0"
    }
}

pub struct CnblogsConfig {
    pub blog_name: String,
    pub username: String,
    pub api_token: String,
}

pub fn read_config(app: &tauri::AppHandle) -> Result<CnblogsConfig, PlatformError> {
    let store = app
        .store("platform_config.json")
        .map_err(|e| PlatformError::not_configured(format!("读取配置失败: {}", e)))?;

    let config = store.get(CONFIG_KEY).map(|v| v.clone()).ok_or_else(|| {
        PlatformError::not_configured("博客园未配置，请先在设置中填写凭据")
    })?;

    let blog_name = config
        .get("blogName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let username = config
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let api_token = config
        .get("apiToken")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let mut missing: Vec<&str> = Vec::new();
    if blog_name.is_empty() {
        missing.push("博客名");
    }
    if username.is_empty() {
        missing.push("用户名");
    }
    if api_token.is_empty() {
        missing.push("访问令牌");
    }
    if !missing.is_empty() {
        return Err(PlatformError::not_configured(format!(
            "博客园凭据不完整，缺少: {}",
            missing.join("、")
        )));
    }

    Ok(CnblogsConfig {
        blog_name,
        username,
        api_token,
    })
}

pub fn build_request_body(
    config: &CnblogsConfig,
    request: &PublishRequest,
    operation: PublishOperation,
) -> Result<String, PlatformError> {
    let method_name = match operation {
        PublishOperation::Update => "metaWeblog.editPost",
        PublishOperation::Create => "metaWeblog.newPost",
    };

    let post_id_param = match operation {
        PublishOperation::Update => {
            let existing_id = request
                .existing_id
                .as_deref()
                .map(|s| s.trim())
                .unwrap_or("");
            if existing_id.is_empty() {
                return Err(PlatformError::not_configured(
                    "更新已发布文章需要已有的文章 ID，但当前映射记录缺少该 ID",
                ));
            }
            format!(
                "<param><value><string>{}</string></value></param>",
                xml_escape(existing_id)
            )
        }
        PublishOperation::Create => {
            format!(
                "<param><value><string>{}</string></value></param>",
                xml_escape(&config.blog_name)
            )
        }
    };

    let boolean_value = publish_boolean(request.publish_status);

    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<methodCall>\
  <methodName>{method_name}</methodName>\
  <params>\
    {post_id_param}\
    <param><value><string>{username}</string></value></param>\
    <param><value><string>{token}</string></value></param>\
    <param><value><struct>\
      <member><name>title</name><value><string>{title}</string></value></member>\
      <member><name>description</name><value><string>{content}</string></value></member>\
      <member><name>categories</name><value><array><data>\
        <value><string>OpenPub</string></value>\
      </data></array></value></member>\
    </struct></value></param>\
    <param><value><boolean>{boolean_value}</boolean></value></param>\
  </params>\
</methodCall>",
        method_name = method_name,
        post_id_param = post_id_param,
        username = xml_escape(&config.username),
        token = xml_escape(&config.api_token),
        title = xml_escape(&request.title),
        content = xml_escape(&request.content),
        boolean_value = boolean_value,
    );

    Ok(body)
}

pub async fn publish(
    app: &tauri::AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    let config = match read_config(app) {
        Ok(c) => c,
        Err(e) => return PublishResult::failure(e.class, e.message, operation, false),
    };

    let body = match build_request_body(&config, request, operation) {
        Ok(b) => b,
        Err(e) => return PublishResult::failure(e.class, e.message, operation, false),
    };

    let endpoint_url = format!("https://rpc.cnblogs.com/metaweblog/{}", config.blog_name);

    let client = reqwest::Client::new();
    let response = match client
        .post(&endpoint_url)
        .header("Content-Type", "text/xml; charset=utf-8")
        .header("User-Agent", "OpenPub/0.1.0")
        .body(bytes::Bytes::from(body))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return PublishResult::failure(
                FailureClass::NetworkError,
                format!("请求发送失败: {}", e),
                operation,
                false,
            )
        }
    };

    let status = response.status();
    let response_text = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            return PublishResult::failure(
                FailureClass::NetworkError,
                format!("读取响应失败: {}", e),
                operation,
                false,
            )
        }
    };

    if response_text.contains("<fault>") {
        let fault_start = response_text.find("<fault>").unwrap_or(0);
        let fault_end = response_text
            .find("</fault>")
            .map(|i| i + "</fault>".len())
            .unwrap_or(response_text.len());
        let fault_section = &response_text[fault_start..fault_end.min(response_text.len())];
        return PublishResult::failure(
            FailureClass::PlatformError,
            format!("MetaWeblog 错误 (HTTP {}): {}", status, fault_section),
            operation,
            false,
        );
    }

    if !status.is_success() {
        return PublishResult::failure(
            FailureClass::PlatformError,
            format!(
                "HTTP {}, 响应: {}",
                status,
                truncate_chars(&response_text, ERROR_SNIPPET_CHARS)
            ),
            operation,
            request.publish_status,
        );
    }

    let post_id = response_text
        .find("<string>")
        .and_then(|start| {
            let end = response_text[start..].find("</string>")?;
            Some(response_text[start + 8..start + end].to_string())
        })
        .unwrap_or_else(|| "(未知)".to_string());

    let edit_url = format!("https://i.cnblogs.com/posts/edit;postId={}", post_id);
    let action = match operation {
        PublishOperation::Update => "更新",
        PublishOperation::Create => "发布",
    };
    let target = if request.publish_status {
        "并已上线"
    } else {
        "并保存为草稿"
    };

    PublishResult::success(
        post_id.clone(),
        edit_url,
        format!("{}成功{}! 文章 ID: {}", action, target, post_id),
        operation,
        request.publish_status,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> CnblogsConfig {
        CnblogsConfig {
            blog_name: "my-blog".to_string(),
            username: "user".to_string(),
            api_token: "token".to_string(),
        }
    }

    fn sample_request(publish_status: bool, is_update: bool, existing_id: Option<&str>) -> PublishRequest {
        PublishRequest {
            title: "标题".to_string(),
            content: "# 正文".to_string(),
            existing_id: existing_id.map(|s| s.to_string()),
            is_update,
            publish_status,
            existing_published: false,
        }
    }

    #[test]
    fn draft_only_request_sends_false_boolean() {
        let body = build_request_body(
            &sample_config(),
            &sample_request(false, false, None),
            PublishOperation::Create,
        )
        .expect("body should build");
        assert!(body.contains("<boolean>0</boolean>"));
        assert!(!body.contains("<boolean>1</boolean>"));
    }

    #[test]
    fn live_request_sends_true_boolean() {
        let body = build_request_body(
            &sample_config(),
            &sample_request(true, false, None),
            PublishOperation::Create,
        )
        .expect("body should build");
        assert!(body.contains("<boolean>1</boolean>"));
    }

    #[test]
    fn create_path_uses_new_post() {
        let body = build_request_body(
            &sample_config(),
            &sample_request(false, false, None),
            PublishOperation::Create,
        )
        .expect("body should build");
        assert!(body.contains("metaWeblog.newPost"));
        assert!(!body.contains("metaWeblog.editPost"));
    }

    #[test]
    fn update_path_uses_edit_post_with_existing_id() {
        let body = build_request_body(
            &sample_config(),
            &sample_request(false, true, Some("12345")),
            PublishOperation::Update,
        )
        .expect("body should build");
        assert!(body.contains("metaWeblog.editPost"));
        assert!(!body.contains("metaWeblog.newPost"));
        assert!(body.contains("<string>12345</string>"));
    }

    #[test]
    fn update_without_existing_id_is_rejected() {
        let result = build_request_body(
            &sample_config(),
            &sample_request(true, true, None),
            PublishOperation::Update,
        );
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert_eq!(err.class, FailureClass::NotConfigured);
    }

    #[test]
    fn update_with_empty_existing_id_is_rejected() {
        let result = build_request_body(
            &sample_config(),
            &sample_request(false, true, Some("   ")),
            PublishOperation::Update,
        );
        assert!(result.is_err());
    }

    #[test]
    fn update_live_request_combines_edit_post_and_true_boolean() {
        let body = build_request_body(
            &sample_config(),
            &sample_request(true, true, Some("999")),
            PublishOperation::Update,
        )
        .expect("body should build");
        assert!(body.contains("metaWeblog.editPost"));
        assert!(body.contains("<boolean>1</boolean>"));
        assert!(body.contains("<string>999</string>"));
    }

    #[test]
    fn xml_special_characters_are_escaped() {
        let mut request = sample_request(false, false, None);
        request.title = "A & B <C>".to_string();
        let body = build_request_body(&sample_config(), &request, PublishOperation::Create)
            .expect("body should build");
        assert!(body.contains("A &amp; B &lt;C&gt;"));
        assert!(!body.contains("<C>"));
    }

    #[test]
    fn truncate_chars_handles_multibyte_without_panic() {
        let long = "中".repeat(400);
        let truncated = truncate_chars(&long, 10);
        assert!(truncated.chars().count() < long.chars().count());
    }

    #[test]
    fn fault_response_is_classified_as_platform_error() {
        let err = PlatformError::platform_error("fault");
        assert_eq!(err.class, FailureClass::PlatformError);
    }
}