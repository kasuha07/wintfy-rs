use std::{
    io::{BufReader, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{
    config::SubscriptionConfig,
    ntfy::{auth, event},
};

#[derive(Debug)]
pub enum StreamItem {
    Open,
    Keepalive,
    Message(event::NtfyEvent),
    PollRequest,
    InvalidJson(String),
    OversizeLine(usize),
}

#[derive(Debug)]
pub struct StreamError {
    pub message: String,
    pub slow_backoff: bool,
}

impl StreamError {
    pub fn new(message: impl Into<String>, slow_backoff: bool) -> Self {
        Self {
            message: message.into(),
            slow_backoff,
        }
    }
}

pub fn subscription_url(sub: &SubscriptionConfig) -> Result<String, String> {
    let mut base = url::Url::parse(&sub.server)
        .map_err(|err| format!("invalid server URL for {}: {err}", sub.name))?;
    let topics = sub
        .topics
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(",");

    let mut path = base.path().trim_end_matches('/').to_string();
    path.push('/');
    path.push_str(&topics);
    path.push_str("/json");
    base.set_path(&path);
    base.set_query(None);
    Ok(base.to_string())
}

pub fn read_stream<F>(
    agent: &ureq::Agent,
    sub: &SubscriptionConfig,
    version: &str,
    line_max_bytes: usize,
    shutdown: &Arc<AtomicBool>,
    on_item: F,
) -> Result<(), StreamError>
where
    F: FnMut(StreamItem),
{
    let url = subscription_url(sub).map_err(|err| StreamError::new(err, false))?;
    let mut request = agent
        .get(&url)
        .set("User-Agent", &format!("wintfy-rs/{version}"))
        .set("Accept", "application/x-ndjson, application/json, */*");
    if let Some(header) = auth::auth_header(sub) {
        request = request.set("Authorization", &header);
    }

    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let slow = matches!(code, 401 | 403 | 404 | 429);
            let server = response.get_url().to_string();
            return Err(StreamError::new(
                format!(
                    "HTTP {code} from {}",
                    crate::util::redact::redact_secret(&server)
                ),
                slow,
            ));
        }
        Err(err) => return Err(StreamError::new(format!("request failed: {err}"), false)),
    };

    let reader = response.into_reader();
    read_json_lines(reader, line_max_bytes, shutdown, on_item)
}

pub fn read_json_lines<R, F>(
    reader: R,
    line_max_bytes: usize,
    shutdown: &Arc<AtomicBool>,
    mut on_item: F,
) -> Result<(), StreamError>
where
    R: Read,
    F: FnMut(StreamItem),
{
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::with_capacity(4096);

    while !shutdown.load(Ordering::Relaxed) {
        match read_limited_line(&mut reader, &mut bytes, line_max_bytes) {
            Ok(LineRead::Eof) => return Err(StreamError::new("stream ended", false)),
            Ok(LineRead::Oversize(n)) => {
                on_item(StreamItem::OversizeLine(n));
                continue;
            }
            Ok(LineRead::Line) => {
                while matches!(bytes.last(), Some(b'\n' | b'\r')) {
                    bytes.pop();
                }
                if bytes.is_empty() {
                    continue;
                }
                let line = match std::str::from_utf8(&bytes) {
                    Ok(line) => line,
                    Err(err) => {
                        on_item(StreamItem::InvalidJson(format!("invalid utf-8: {err}")));
                        continue;
                    }
                };
                match event::parse_line(line) {
                    Ok(event) => match event.event.as_str() {
                        "open" => on_item(StreamItem::Open),
                        "keepalive" => on_item(StreamItem::Keepalive),
                        "message" => on_item(StreamItem::Message(event)),
                        "poll_request" => on_item(StreamItem::PollRequest),
                        _ => on_item(StreamItem::InvalidJson(format!(
                            "unknown event type: {}",
                            event.event
                        ))),
                    },
                    Err(err) => on_item(StreamItem::InvalidJson(err.to_string())),
                }
            }
            Err(err) => return Err(StreamError::new(format!("read failed: {err}"), false)),
        }
    }

    Ok(())
}

enum LineRead {
    Line,
    Oversize(usize),
    Eof,
}

fn read_limited_line<R: Read>(
    reader: &mut BufReader<R>,
    bytes: &mut Vec<u8>,
    line_max_bytes: usize,
) -> std::io::Result<LineRead> {
    bytes.clear();
    let mut total = 0usize;
    let mut byte = [0u8; 1];

    loop {
        match reader.read(&mut byte)? {
            0 => {
                return Ok(if bytes.is_empty() {
                    LineRead::Eof
                } else {
                    LineRead::Line
                });
            }
            1 => {
                total = total.saturating_add(1);
                if total > line_max_bytes {
                    while reader.read(&mut byte)? == 1 {
                        total = total.saturating_add(1);
                        if byte[0] == b'\n' {
                            break;
                        }
                    }
                    bytes.clear();
                    return Ok(LineRead::Oversize(total));
                }
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    return Ok(LineRead::Line);
                }
            }
            _ => unreachable!(),
        }
    }
}

pub fn agent() -> Result<ureq::Agent, String> {
    let tls = ureq::native_tls::TlsConnector::new()
        .map_err(|err| format!("native TLS initialization failed: {err}"))?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_read(Duration::from_secs(90))
        .timeout_write(Duration::from_secs(30))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthKind;
    use crate::config::SubscriptionConfig;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{atomic::AtomicBool, mpsc},
        thread,
    };

    #[test]
    fn builds_url_with_path_prefix() {
        let sub = SubscriptionConfig {
            name: "x".to_string(),
            server: "https://example.com/ntfy/".to_string(),
            topics: vec!["a".to_string(), "b".to_string()],
            ..Default::default()
        };
        assert_eq!(
            subscription_url(&sub).unwrap(),
            "https://example.com/ntfy/a,b/json"
        );
    }

    #[test]
    fn reads_json_lines() {
        let input = br#"{"event":"open"}
{"event":"keepalive"}
{"event":"message","id":"1","topic":"a","message":"hello"}
"#;
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut items = Vec::new();
        let result = read_json_lines(&input[..], 1024, &shutdown, |item| items.push(item));
        assert!(result.is_err());
        assert!(matches!(items[0], StreamItem::Open));
        assert!(matches!(items[1], StreamItem::Keepalive));
        assert!(matches!(items[2], StreamItem::Message(_)));
    }

    #[test]
    fn reports_oversize_line() {
        let input = br#"{"event":"keepalive"}
"#;
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut items = Vec::new();
        let _ = read_json_lines(&input[..], 4, &shutdown, |item| items.push(item));
        assert!(matches!(items[0], StreamItem::OversizeLine(_)));
    }

    #[test]
    fn continues_after_invalid_and_oversize_lines() {
        let input = b"{not-json}\n{\"event\":\"keepalive\",\"padding\":\"xxxxxxxx\"}\n{\"event\":\"message\"}\n";
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut items = Vec::new();
        let result = read_json_lines(&input[..], 32, &shutdown, |item| items.push(item));
        assert!(result.is_err());
        assert!(matches!(items[0], StreamItem::InvalidJson(_)));
        assert!(matches!(items[1], StreamItem::OversizeLine(_)));
        assert!(matches!(items[2], StreamItem::Message(_)));
    }

    #[test]
    fn read_stream_sends_bearer_and_reads_message() {
        let (url, request_rx, server) = spawn_one_response(
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n{\"event\":\"open\"}\n{\"event\":\"message\",\"id\":\"1\",\"topic\":\"smoke\",\"message\":\"hi\"}\n",
        );
        let sub = SubscriptionConfig {
            name: "private".to_string(),
            server: url,
            topics: vec!["smoke".to_string()],
            auth: AuthKind::Bearer,
            token: Some("tk_test".to_string()),
            ..Default::default()
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut items = Vec::new();
        let agent = agent().unwrap();
        let result = read_stream(&agent, &sub, "test", 1024, &shutdown, |item| {
            items.push(item)
        });
        let request = request_rx.recv().unwrap();
        server.join().unwrap();
        assert!(request.contains("GET /smoke/json HTTP/1.1"));
        assert!(
            request.contains("authorization: Bearer tk_test")
                || request.contains("Authorization: Bearer tk_test")
        );
        assert!(result.is_err());
        assert!(matches!(items[0], StreamItem::Open));
        assert!(matches!(items[1], StreamItem::Message(_)));
    }

    #[test]
    fn read_stream_marks_401_as_slow_backoff() {
        let (url, _request_rx, server) = spawn_one_response(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let sub = SubscriptionConfig {
            name: "private".to_string(),
            server: url,
            topics: vec!["smoke".to_string()],
            ..Default::default()
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let agent = agent().unwrap();
        let err = read_stream(&agent, &sub, "test", 1024, &shutdown, |_| {}).unwrap_err();
        server.join().unwrap();
        assert!(err.slow_backoff);
        assert!(err.message.contains("HTTP 401"));
    }

    fn spawn_one_response(
        response: &'static str,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            tx.send(String::from_utf8_lossy(&request).to_string())
                .unwrap();
            stream.write_all(response.as_bytes()).unwrap();
        });
        (format!("http://{addr}"), rx, handle)
    }
}
