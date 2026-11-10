//! 单测用的本地 HTTP 服务。
//!
//! 纯 Cookie 直发平台的单测需要真实走一遍「发请求 -> 读响应 -> 分类」链路，
//! 但不应依赖外网。这里用 `TcpListener` 起一个最小 HTTP 服务，按
//! `"METHOD /path"` 前缀路由返回预置响应，测试结束即关闭。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct MockServer {
    pub base: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockServer {
    /// `routes` 按顺序匹配，先命中者生效；`fallback` 用于未命中的请求。
    pub fn start(routes: Vec<(&str, u16, &str)>, fallback: (u16, &str)) -> Self {
        Self::start_with_headers(routes, fallback, &[])
    }

    /// 同上，但可以附加响应头。CSRF 探针（`x-ware-csrf-token`）只能从响应头取值，
    /// 因此头条一类的预检单测必须能自定义头。
    pub fn start_with_headers(
        routes: Vec<(&str, u16, &str)>,
        fallback: (u16, &str),
        headers: &[(&str, &str)],
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server bind");
        listener.set_nonblocking(true).expect("mock server nonblocking");
        let address = listener.local_addr().expect("mock server address");

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        let routes: Vec<(String, u16, String)> = routes
            .into_iter()
            .map(|(key, status, body)| (key.to_string(), status, body.to_string()))
            .collect();
        let fallback = (fallback.0, fallback.1.to_string());
        let headers: Vec<(String, String)> = headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();

        let handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve(&mut stream, &routes, &fallback, &headers),
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        });

        Self {
            base: format!("http://{}", address),
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(
    stream: &mut TcpStream,
    routes: &[(String, u16, String)],
    fallback: &(u16, String),
    headers: &[(String, String)],
) {
    // 监听器是非阻塞的，部分平台（Windows）会把非阻塞标志遗传给 accept 出来的流，
    // 这会让读写立即返回 WouldBlock，客户端收到半个响应而误判成网络错误。
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
    let request = read_request(stream);
    let (method, path) = parse_request_line(&request);

    let matched = routes.iter().find(|(key, _, _)| {
        let mut parts = key.splitn(2, ' ');
        let route_method = parts.next().unwrap_or("");
        let route_path = parts.next().unwrap_or("");
        method == route_method && path.starts_with(route_path)
    });

    let (status, mut body) = match matched {
        Some((_, status, body)) => (*status, body.clone()),
        None => fallback.clone(),
    };

    // HEAD 只读响应头，带上 body 会让客户端误判长度，这里按协议清空。
    if method == "HEAD" {
        body.clear();
    }

    let extra_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{}: {}\r\n", name, value))
        .collect();

    let response = format!(
        "HTTP/1.1 {} Mock\r\nContent-Type: application/json; charset=utf-8\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        extra_headers,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if let Some(header_end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header_len = header_end + 4;
                    let content_length = content_length_of(&buffer[..header_len]);
                    if buffer.len() >= header_len + content_length {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }

    String::from_utf8_lossy(&buffer).to_string()
}

fn content_length_of(headers: &[u8]) -> usize {
    let text = String::from_utf8_lossy(headers).to_lowercase();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("content-length:") {
            if let Ok(value) = rest.trim().parse::<usize>() {
                return value;
            }
        }
    }
    0
}

fn parse_request_line(request: &str) -> (&str, &str) {
    let mut parts = request.lines().next().unwrap_or("").split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    (method, path)
}

/// 取一个刚刚释放的本地端口，任何连接都会被拒绝，用于验证 `NetworkError`。
pub fn closed_port_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("closed port bind");
    let address = listener.local_addr().expect("closed port address");
    drop(listener);
    format!("http://{}", address)
}