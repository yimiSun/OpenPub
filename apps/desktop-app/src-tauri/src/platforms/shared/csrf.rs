use super::cookie;
use crate::platforms::FailureClass;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub const CSRF_TOKEN_HEADER: &str = "x-ware-csrf-token";
pub const CSRF_REQUEST_HEADER: &str = "x-secsdk-csrf-request";
pub const CSRF_VERSION_HEADER: &str = "x-secsdk-csrf-version";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSelect {
    /// 通用选区，当前生产路径未用到，保留给后续平台接入。
    #[cfg_attr(not(test), allow(dead_code))]
    Whole,
    /// 按逗号切片取第 N 段，当前生产路径未用到，保留给后续平台接入。
    #[cfg_attr(not(test), allow(dead_code))]
    Segment(usize),
    /// 兼容选区：有逗号取逗号后的部分，没有逗号就用整串。
    ///
    /// 头条探针真实返回形如 `0,实际token`，业务接口要发后半段；但部分环境/测试
    /// 只回单段值，此时取后半段会得到空值。用这个变体同时满足两种形态。
    SegmentAfterComma,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsrfProbeError {
    Network(String),
    Status(u16),
    MissingHeader,
    EmptyToken,
}

impl CsrfProbeError {
    pub fn failure_class(&self) -> FailureClass {
        FailureClass::NotConfigured
    }

    pub fn message(&self) -> String {
        match self {
            CsrfProbeError::Network(detail) => format!("CSRF 探针请求失败: {}", detail),
            CsrfProbeError::Status(code) => format!(
                "CSRF 探针返回状态码 {}，请重新登录后重试",
                code
            ),
            CsrfProbeError::MissingHeader => format!(
                "CSRF 探针响应缺少 {} 头，请重新登录后重试",
                CSRF_TOKEN_HEADER
            ),
            CsrfProbeError::EmptyToken => {
                "CSRF 探针返回的 token 为空，请重新登录后重试".to_string()
            }
        }
    }

    pub fn into_platform_error(self) -> (FailureClass, String) {
        let class = self.failure_class();
        let message = self.message();
        (class, message)
    }
}

impl std::fmt::Display for CsrfProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

pub fn select_token(raw: &str, select: TokenSelect) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let value = match select {
        TokenSelect::Whole => trimmed.to_string(),
        TokenSelect::Segment(index) => trimmed.split(',').nth(index)?.trim().to_string(),
        TokenSelect::SegmentAfterComma => match trimmed.split_once(',') {
            Some((_, second)) => second.trim().to_string(),
            None => trimmed.to_string(),
        },
    };

    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub async fn probe_csrf_token(
    client: &reqwest::Client,
    url: &str,
    cookie: Option<&str>,
    version: &str,
    select: TokenSelect,
) -> Result<String, CsrfProbeError> {
    let mut request = client
        .head(url)
        .header("User-Agent", USER_AGENT)
        .header(CSRF_REQUEST_HEADER, "1");

    let version = version.trim();
    if !version.is_empty() {
        request = request.header(CSRF_VERSION_HEADER, version);
    }

    if let Some(cookie) = cookie {
        let cookie = cookie.trim();
        if !cookie.is_empty() {
            request = request.header("Cookie", cookie);
        }
    }

    let response = request
        .send()
        .await
        .map_err(|error| CsrfProbeError::Network(error.to_string()))?;

    let status = response.status();
    if status != reqwest::StatusCode::OK {
        return Err(CsrfProbeError::Status(status.as_u16()));
    }

    let raw = response
        .headers()
        .get(CSRF_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(CsrfProbeError::MissingHeader)?;

    select_token(raw, select).ok_or(CsrfProbeError::EmptyToken)
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(h), Some(l)) = (high, low) {
                output.push(h * 16 + l);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&output).to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn extract_token(cookie: &str, key: &str) -> Option<String> {
    cookie::extract_value(cookie, key).map(|raw| percent_decode(&raw))
}

pub fn require_token(cookie: &str, key: &str, platform: &str) -> Result<String, String> {
    extract_token(cookie, key)
        .ok_or_else(|| format!("Cookie 中缺少 {}，请重新登录 {} 后复制完整 Cookie", key, platform))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_encoded_token() {
        assert_eq!(percent_decode("abc%3D%3D"), "abc==");
    }

    #[test]
    fn leaves_unencoded_token_untouched() {
        assert_eq!(percent_decode("abcDEF123"), "abcDEF123");
    }

    #[test]
    fn decodes_lowercase_hex_escapes() {
        assert_eq!(percent_decode("%2f%2F"), "//");
    }

    #[test]
    fn keeps_invalid_escape_sequences_literal() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn extracts_and_decodes_token_from_cookie() {
        let cookie = "a=1; _xsrf=abc%3D%3D; b=2";
        assert_eq!(extract_token(cookie, "_xsrf"), Some("abc==".to_string()));
    }

    #[test]
    fn missing_token_reports_key_by_name() {
        let error = require_token("a=1", "_xsrf", "知乎").unwrap_err();
        assert!(error.contains("_xsrf"));
        assert!(error.contains("知乎"));
    }

    #[test]
    fn whole_select_keeps_full_value() {
        assert_eq!(
            select_token("  abc,def  ", TokenSelect::Whole),
            Some("abc,def".to_string())
        );
    }

    #[test]
    fn segment_select_takes_nth_part() {
        assert_eq!(
            select_token("head,payload,tail", TokenSelect::Segment(1)),
            Some("payload".to_string())
        );
    }

    #[test]
    fn segment_select_out_of_range_is_none() {
        assert_eq!(select_token("only", TokenSelect::Segment(1)), None);
    }

    #[test]
    fn segment_after_comma_takes_second_part() {
        assert_eq!(
            select_token("0,real-token", TokenSelect::SegmentAfterComma),
            Some("real-token".to_string())
        );
    }

    #[test]
    fn segment_after_comma_falls_back_to_whole_when_no_comma() {
        assert_eq!(
            select_token("single-token", TokenSelect::SegmentAfterComma),
            Some("single-token".to_string())
        );
    }

    #[test]
    fn segment_after_comma_keeps_rest_after_first_comma() {
        assert_eq!(
            select_token("a,b,c", TokenSelect::SegmentAfterComma),
            Some("b,c".to_string())
        );
    }

    #[test]
    fn empty_raw_value_is_none() {
        assert_eq!(select_token("   ", TokenSelect::Whole), None);
    }

    #[test]
    fn every_probe_error_maps_to_not_configured() {
        let errors = [
            CsrfProbeError::Network("timed out".to_string()),
            CsrfProbeError::Status(404),
            CsrfProbeError::MissingHeader,
            CsrfProbeError::EmptyToken,
        ];
        for error in errors {
            assert_eq!(error.failure_class(), FailureClass::NotConfigured);
        }
    }

    fn serve_once(response: &'static str) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buffer = [0u8; 1024];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://{}", address), handle)
    }

    #[test]
    fn missing_header_probe_is_not_configured_and_not_retried() {
        let (url, handle) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let client = reqwest::Client::new();
        let error = tauri::async_runtime::block_on(probe_csrf_token(
            &client,
            &url,
            None,
            "1.2.22",
            TokenSelect::Whole,
        ))
        .unwrap_err();
        assert_eq!(error, CsrfProbeError::MissingHeader);
        assert_eq!(error.failure_class(), FailureClass::NotConfigured);
        handle.join().unwrap();
    }

    #[test]
    fn header_value_is_selected_after_probe() {
        let (url, handle) = serve_once(
            "HTTP/1.1 200 OK\r\nx-ware-csrf-token: head,payload\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let client = reqwest::Client::new();
        let token = tauri::async_runtime::block_on(probe_csrf_token(
            &client,
            &url,
            Some("a=1"),
            "1.2.10",
            TokenSelect::Segment(1),
        ))
        .unwrap();
        assert_eq!(token, "payload");
        handle.join().unwrap();
    }

    #[test]
    fn timeout_probe_is_not_configured() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            if let Ok((_stream, _)) = listener.accept() {
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
        });
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(300))
            .build()
            .unwrap();
        let error = tauri::async_runtime::block_on(probe_csrf_token(
            &client,
            &format!("http://{}", address),
            None,
            "1.2.22",
            TokenSelect::Whole,
        ))
        .unwrap_err();
        assert!(matches!(error, CsrfProbeError::Network(_)));
        assert_eq!(error.failure_class(), FailureClass::NotConfigured);
        handle.join().unwrap();
    }
}