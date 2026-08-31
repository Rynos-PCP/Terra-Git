//! Minimal HTTP/1.1 test server (std only): answers requests based on path
//! prefix routes and records method/path/headers for assertions.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    /// Path including the query string.
    pub path: String,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }
}

pub struct TestServer {
    pub base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl TestServer {
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

/// Starts the server on a free port. `routes` = (path prefix, status, JSON
/// body); the first matching route wins, otherwise 404.
pub fn start(routes: Vec<(&'static str, u16, &'static str)>) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::default();
    let recorded = requests.clone();

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // Read the request head (the body is irrelevant for GET tests).
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => buf.push(byte[0]),
                    _ => break,
                }
            }
            let head = String::from_utf8_lossy(&buf);
            let mut lines = head.lines();
            let request_line = lines.next().unwrap_or_default();
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let path = parts.next().unwrap_or_default().to_string();
            let headers = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(n, v)| (n.trim().to_lowercase(), v.trim().to_string()))
                .collect();
            recorded.lock().unwrap().push(RecordedRequest {
                method,
                path: path.clone(),
                headers,
            });

            let bare_path = path.split('?').next().unwrap_or(&path);
            let (status, body) = routes
                .iter()
                .find(|(prefix, _, _)| bare_path.starts_with(prefix))
                .map(|(_, s, b)| (*s, *b))
                .unwrap_or((404, r#"{"message":"not found"}"#));
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    TestServer { base_url, requests }
}
