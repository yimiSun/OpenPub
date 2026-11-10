//! 纯 Cookie 直发平台的通用响应分类。
//!
//! 八个平台的 HTTP 语义高度同构：`401/403` 是登录态失效，其余非 2xx 是平台异常，
//! 网络层失败是 `NetworkError`。把这层判断集中在这里，避免每个平台各写一份
//! 分类逻辑后又各自漂移。

use super::truncate::truncate_chars;
use crate::platforms::{FailureClass, PublishOperation, PublishResult};

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub const ERROR_SNIPPET_CHARS: usize = 300;

pub fn new_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

pub fn is_auth_status(status: u16) -> bool {
    matches!(status, 401 | 403)
}

pub fn snippet(body: &str) -> String {
    truncate_chars(body.trim(), ERROR_SNIPPET_CHARS)
}

pub fn network_failure(
    platform: &str,
    detail: &str,
    operation: PublishOperation,
) -> PublishResult {
    PublishResult::failure(
        FailureClass::NetworkError,
        format!("{} 请求失败: {}", platform, detail),
        operation,
        false,
    )
}

pub fn auth_failure(
    platform: &str,
    detail: &str,
    operation: PublishOperation,
    is_published: bool,
) -> PublishResult {
    PublishResult::failure(
        FailureClass::AuthInvalid,
        format!("{} 登录态已失效: {}", platform, detail),
        operation,
        is_published,
    )
}

pub fn http_failure(
    platform: &str,
    status: u16,
    body: &str,
    operation: PublishOperation,
    is_published: bool,
) -> PublishResult {
    if is_auth_status(status) {
        return auth_failure(platform, &format!("HTTP {}", status), operation, is_published);
    }

    PublishResult::failure(
        FailureClass::PlatformError,
        format!(
            "{} 接口返回异常（HTTP {}）: {}",
            platform,
            status,
            snippet(body)
        ),
        operation,
        is_published,
    )
}

pub fn business_failure(
    platform: &str,
    message: impl AsRef<str>,
    operation: PublishOperation,
    is_published: bool,
) -> PublishResult {
    PublishResult::failure(
        FailureClass::PlatformError,
        format!("{}: {}", platform, message.as_ref()),
        operation,
        is_published,
    )
}

pub async fn read_text(
    result: Result<reqwest::Response, reqwest::Error>,
    platform: &str,
    operation: PublishOperation,
) -> Result<(u16, String), PublishResult> {
    match result {
        Ok(response) => {
            let status = response.status().as_u16();
            match response.text().await {
                Ok(body) => Ok((status, body)),
                Err(error) => Err(network_failure(
                    platform,
                    &format!("读取响应失败: {}", error),
                    operation,
                )),
            }
        }
        Err(error) => Err(network_failure(
            platform,
            &format!("请求发送失败: {}", error),
            operation,
        )),
    }
}

/// 同 `read_text`，但把响应头与**最终 URL**一并返回。
///
/// 少数平台的发布凭证既不在页面 HTML 里，也不在响应头里，而是登录后由服务端
/// 用 302 重定向塞进 URL 查询串（百家号 `/builder/rc/edit` 就是跳到
/// `...?type=news&token=xxx`）。HTML 里的正则随前端改版一改就失效，URL 与
/// 响应头反而更稳定，因此这类平台必须能读到最终 URL。
pub async fn read_text_with_headers(
    result: Result<reqwest::Response, reqwest::Error>,
    platform: &str,
    operation: PublishOperation,
) -> Result<TextResponse, PublishResult> {
    match result {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();
            match response.text().await {
                Ok(body) => Ok(TextResponse {
                    status,
                    body,
                    headers,
                    final_url,
                }),
                Err(error) => Err(network_failure(
                    platform,
                    &format!("读取响应失败: {}", error),
                    operation,
                )),
            }
        }
        Err(error) => Err(network_failure(
            platform,
            &format!("请求发送失败: {}", error),
            operation,
        )),
    }
}

pub struct TextResponse {
    pub status: u16,
    pub body: String,
    pub headers: reqwest::header::HeaderMap,
    pub final_url: String,
}

/// 按候选键名在响应头里取值（不区分大小写，`HeaderMap` 本身即忽略大小写）。
pub fn header_value(headers: &reqwest::header::HeaderMap, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = headers.get(*key).and_then(|v| v.to_str().ok()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub fn parse_json(body: &str) -> Option<serde_json::Value> {
    serde_json::from_str(body).ok()
}