use super::cookie;

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
}