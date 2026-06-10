use std::{
    io::{BufRead, BufReader, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    config::SubscriptionConfig,
    ntfy::{auth, event},
    util::url,
};

#[cfg(any(feature = "winhttp", feature = "native-tls-client"))]
use crate::util::url::ParsedUrl;

#[cfg(any(feature = "winhttp", feature = "native-tls-client"))]
use std::fmt::Write as _;

#[cfg(any(
    feature = "native-tls-client",
    all(
        feature = "ureq-client",
        not(any(feature = "winhttp", feature = "native-tls-client"))
    )
))]
use std::time::Duration;

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum StreamItem {
    Open,
    Keepalive,
    Message(event::NtfyEvent),
    PollRequest,
    InvalidJson(String),
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
    url::build_stream_url(&sub.server, &sub.topics)
        .map_err(|err| format!("invalid server URL for {}: {err}", sub.name))
}

pub fn user_agent() -> &'static str {
    concat!("wintfy-rs/", env!("CARGO_PKG_VERSION"))
}

pub fn read_stream<F>(
    client: &HttpClient,
    sub: &SubscriptionConfig,
    user_agent: &str,
    line_max_bytes: usize,
    shutdown: &Arc<AtomicBool>,
    on_item: F,
) -> Result<(), StreamError>
where
    F: FnMut(StreamItem),
{
    let reader = client.open(sub, user_agent)?;
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
                on_item(StreamItem::InvalidJson(format!(
                    "stream line too large ({n} bytes)"
                )));
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

fn read_limited_line<R: BufRead>(
    reader: &mut R,
    bytes: &mut Vec<u8>,
    line_max_bytes: usize,
) -> std::io::Result<LineRead> {
    bytes.clear();
    let mut total = 0usize;
    let mut oversize = false;

    loop {
        let (consume, found_newline) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(if total == 0 {
                    LineRead::Eof
                } else if oversize {
                    LineRead::Oversize(total)
                } else {
                    LineRead::Line
                });
            }

            let consume = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|pos| pos + 1)
                .unwrap_or(available.len());
            total = total.saturating_add(consume);

            if !oversize {
                if total > line_max_bytes {
                    bytes.clear();
                    oversize = true;
                } else {
                    bytes.extend_from_slice(&available[..consume]);
                }
            }

            (
                consume,
                consume < available.len() || available[consume - 1] == b'\n',
            )
        };
        reader.consume(consume);
        if found_newline {
            return Ok(if oversize {
                LineRead::Oversize(total)
            } else {
                LineRead::Line
            });
        }
    }
}

pub fn agent(skip_tls_verify: bool) -> Result<HttpClient, String> {
    HttpClient::new(skip_tls_verify)
}

pub struct HttpClient {
    backend: BackendClient,
}

impl HttpClient {
    fn new(skip_tls_verify: bool) -> Result<Self, String> {
        Ok(Self {
            backend: BackendClient::new(skip_tls_verify)?,
        })
    }

    fn open(&self, sub: &SubscriptionConfig, user_agent: &str) -> Result<HttpStream, StreamError> {
        self.backend.open(sub, user_agent)
    }
}

#[cfg(all(feature = "winhttp", not(feature = "native-tls-client")))]
struct BackendClient {
    session: winhttp::Session,
    skip_tls_verify: bool,
}

#[cfg(all(feature = "winhttp", not(feature = "native-tls-client")))]
impl BackendClient {
    fn new(skip_tls_verify: bool) -> Result<Self, String> {
        Ok(Self {
            session: winhttp::Session::new(user_agent()).map_err(|err| err.message)?,
            skip_tls_verify,
        })
    }

    fn open(&self, sub: &SubscriptionConfig, user_agent: &str) -> Result<HttpStream, StreamError> {
        self.session
            .open_stream(sub, user_agent, self.skip_tls_verify)
    }
}

#[cfg(feature = "native-tls-client")]
struct BackendClient {
    tls: native_tls::TlsConnector,
    skip_tls_verify: bool,
}

#[cfg(feature = "native-tls-client")]
impl BackendClient {
    fn new(skip_tls_verify: bool) -> Result<Self, String> {
        let mut tls = native_tls::TlsConnector::builder();
        if skip_tls_verify {
            tls.danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
        Ok(Self {
            tls: tls
                .build()
                .map_err(|err| format!("native TLS initialization failed: {err}"))?,
            skip_tls_verify,
        })
    }

    fn open(&self, sub: &SubscriptionConfig, user_agent: &str) -> Result<HttpStream, StreamError> {
        direct::open_stream(sub, user_agent, &self.tls, self.skip_tls_verify)
    }
}

#[cfg(all(
    feature = "ureq-client",
    not(any(feature = "winhttp", feature = "native-tls-client"))
))]
struct BackendClient {
    agent: ureq::Agent,
}

#[cfg(all(
    feature = "ureq-client",
    not(any(feature = "winhttp", feature = "native-tls-client"))
))]
impl BackendClient {
    fn new(skip_tls_verify: bool) -> Result<Self, String> {
        let mut tls = ureq::native_tls::TlsConnector::builder();
        if skip_tls_verify {
            tls.danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
        let tls = tls
            .build()
            .map_err(|err| format!("native TLS initialization failed: {err}"))?;
        Ok(Self {
            agent: ureq::AgentBuilder::new()
                .tls_connector(Arc::new(tls))
                .timeout_read(Duration::from_secs(90))
                .timeout_write(Duration::from_secs(30))
                .build(),
        })
    }

    fn open(&self, sub: &SubscriptionConfig, user_agent: &str) -> Result<HttpStream, StreamError> {
        let url = subscription_url(sub).map_err(|err| StreamError::new(err, false))?;
        let mut request = self
            .agent
            .get(&url)
            .set("User-Agent", user_agent)
            .set("Accept", "application/x-ndjson, application/json, */*");
        if let Some(header) = auth::auth_header(sub) {
            request = request.set("Authorization", &header);
        }

        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) => {
                let slow = slow_status(code.into());
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

        Ok(HttpStream::Ureq(response.into_reader()))
    }
}

#[cfg(not(any(
    feature = "winhttp",
    feature = "native-tls-client",
    feature = "ureq-client"
)))]
struct BackendClient;

#[cfg(not(any(
    feature = "winhttp",
    feature = "native-tls-client",
    feature = "ureq-client"
)))]
impl BackendClient {
    fn new(_skip_tls_verify: bool) -> Result<Self, String> {
        Err(
            "no HTTP backend enabled; enable native-tls-client, winhttp, or ureq-client"
                .to_string(),
        )
    }

    fn open(
        &self,
        _sub: &SubscriptionConfig,
        _user_agent: &str,
    ) -> Result<HttpStream, StreamError> {
        Err(StreamError::new("no HTTP backend enabled", true))
    }
}

pub enum HttpStream {
    #[cfg(all(feature = "winhttp", not(feature = "native-tls-client")))]
    WinHttp(winhttp::WinHttpStream),
    #[cfg(feature = "native-tls-client")]
    Direct(direct::DirectStream),
    #[cfg(all(
        feature = "ureq-client",
        not(any(feature = "winhttp", feature = "native-tls-client"))
    ))]
    Ureq(Box<dyn Read + Send + Sync + 'static>),
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(all(feature = "winhttp", not(feature = "native-tls-client")))]
            HttpStream::WinHttp(stream) => stream.read(buf),
            #[cfg(feature = "native-tls-client")]
            HttpStream::Direct(stream) => stream.read(buf),
            #[cfg(all(
                feature = "ureq-client",
                not(any(feature = "winhttp", feature = "native-tls-client"))
            ))]
            HttpStream::Ureq(stream) => stream.read(buf),
        }
    }
}

#[cfg(any(feature = "winhttp", feature = "native-tls-client"))]
fn request_target(url: &ParsedUrl<'_>) -> String {
    let path = url.path();
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

#[cfg(any(feature = "winhttp", feature = "native-tls-client"))]
fn request_headers(user_agent: &str, sub: &SubscriptionConfig) -> String {
    let mut headers = String::with_capacity(user_agent.len() + 128);
    let _ = write!(
        headers,
        "User-Agent: {user_agent}\r\nAccept: application/x-ndjson, application/json, */*\r\n"
    );
    if let Some(header) = auth::auth_header(sub) {
        let _ = write!(headers, "Authorization: {header}\r\n");
    }
    headers
}

#[cfg(any(
    feature = "winhttp",
    feature = "native-tls-client",
    feature = "ureq-client"
))]
fn slow_status(code: u32) -> bool {
    matches!(code, 401 | 403 | 404 | 429)
}

#[cfg(feature = "native-tls-client")]
mod direct {
    use super::*;
    use std::{
        io,
        io::Write,
        net::{TcpStream, ToSocketAddrs},
    };

    const HTTPS_PORT: u16 = 443;
    const HTTP_PORT: u16 = 80;
    const HEADER_MAX_BYTES: usize = 32 * 1024;

    pub fn open_stream(
        sub: &SubscriptionConfig,
        user_agent: &str,
        tls: &native_tls::TlsConnector,
        _skip_tls_verify: bool,
    ) -> Result<HttpStream, StreamError> {
        let url = subscription_url(sub).map_err(|err| StreamError::new(err, false))?;
        let parsed =
            ParsedUrl::parse(&url).map_err(|err| StreamError::new(err.to_string(), false))?;
        let secure = parsed.scheme().eq_ignore_ascii_case("https");
        let (host, port) = parsed
            .host_port()
            .ok_or_else(|| StreamError::new("server URL is missing host", false))?;
        let port = port.unwrap_or(if secure { HTTPS_PORT } else { HTTP_PORT });
        let target = request_target(&parsed);
        let authority = parsed
            .authority()
            .ok_or_else(|| StreamError::new("server URL is missing host", false))?;
        let headers = request_headers(user_agent, sub);

        let tcp = connect_with_timeout(strip_ipv6_brackets(host), port, Duration::from_secs(30))?;
        tcp.set_read_timeout(Some(Duration::from_secs(90)))
            .map_err(|err| StreamError::new(format!("set read timeout failed: {err}"), false))?;
        tcp.set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|err| StreamError::new(format!("set write timeout failed: {err}"), false))?;
        tcp.set_nodelay(true).ok();

        let stream = if secure {
            let stream = tls
                .connect(strip_ipv6_brackets(host), tcp)
                .map_err(|err| StreamError::new(format!("TLS handshake failed: {err}"), false))?;
            Transport::Tls(Box::new(stream))
        } else {
            Transport::Plain(tcp)
        };

        let mut stream = DirectStream {
            inner: stream,
            chunked: false,
            chunk_remaining: 0,
            eof: false,
        };
        write_request(&mut stream, authority, &target, &headers)?;
        let response = read_response_head(&mut stream)?;
        if response.status != 200 {
            return Err(StreamError::new(
                format!(
                    "HTTP {} from {}",
                    response.status,
                    crate::util::redact::redact_secret(&url)
                ),
                slow_status(response.status),
            ));
        }
        stream.chunked = response.chunked;
        Ok(HttpStream::Direct(stream))
    }

    pub struct DirectStream {
        inner: Transport,
        chunked: bool,
        chunk_remaining: usize,
        eof: bool,
    }

    impl Read for DirectStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.chunked {
                return self.inner.read(buf);
            }
            self.read_chunked(buf)
        }
    }

    impl DirectStream {
        fn read_chunked(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() || self.eof {
                return Ok(0);
            }
            loop {
                if self.chunk_remaining > 0 {
                    let max = buf.len().min(self.chunk_remaining);
                    let n = self.inner.read(&mut buf[..max])?;
                    if n == 0 {
                        return Ok(0);
                    }
                    self.chunk_remaining -= n;
                    if self.chunk_remaining == 0 {
                        read_crlf(&mut self.inner)?;
                    }
                    return Ok(n);
                }

                let size = read_chunk_size(&mut self.inner)?;
                if size == 0 {
                    drain_trailers(&mut self.inner)?;
                    self.eof = true;
                    return Ok(0);
                }
                self.chunk_remaining = size;
            }
        }
    }

    enum Transport {
        Plain(TcpStream),
        Tls(Box<native_tls::TlsStream<TcpStream>>),
    }

    impl Read for Transport {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self {
                Self::Plain(stream) => stream.read(buf),
                Self::Tls(stream) => stream.read(buf),
            }
        }
    }

    impl Write for Transport {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match self {
                Self::Plain(stream) => stream.write(buf),
                Self::Tls(stream) => stream.write(buf),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            match self {
                Self::Plain(stream) => stream.flush(),
                Self::Tls(stream) => stream.flush(),
            }
        }
    }

    struct ResponseHead {
        status: u32,
        chunked: bool,
    }

    fn write_request(
        stream: &mut DirectStream,
        authority: &str,
        target: &str,
        headers: &str,
    ) -> Result<(), StreamError> {
        write!(
            stream.inner,
            "GET {target} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n{headers}\r\n"
        )
        .and_then(|_| stream.inner.flush())
        .map_err(|err| StreamError::new(format!("request write failed: {err}"), false))
    }

    fn read_response_head(stream: &mut DirectStream) -> Result<ResponseHead, StreamError> {
        let mut header = Vec::with_capacity(1024);
        while !header.ends_with(b"\r\n\r\n") {
            if header.len() >= HEADER_MAX_BYTES {
                return Err(StreamError::new("response headers too large", false));
            }
            let mut byte = [0u8; 1];
            let n = stream
                .inner
                .read(&mut byte)
                .map_err(|err| StreamError::new(format!("response read failed: {err}"), false))?;
            if n == 0 {
                return Err(StreamError::new("response ended before headers", false));
            }
            header.push(byte[0]);
        }
        let text = std::str::from_utf8(&header)
            .map_err(|err| StreamError::new(format!("invalid response headers: {err}"), false))?;
        let mut lines = text.split("\r\n");
        let status = parse_status(lines.next().unwrap_or(""))?;
        let mut chunked = false;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("Transfer-Encoding")
                && value
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case("chunked"))
            {
                chunked = true;
            }
        }
        Ok(ResponseHead { status, chunked })
    }

    fn parse_status(line: &str) -> Result<u32, StreamError> {
        let mut parts = line.split_whitespace();
        let version = parts.next().unwrap_or("");
        let code = parts
            .next()
            .ok_or_else(|| StreamError::new("missing HTTP status code", false))?;
        if !version.starts_with("HTTP/") {
            return Err(StreamError::new("invalid HTTP response", false));
        }
        code.parse::<u32>()
            .map_err(|err| StreamError::new(format!("invalid HTTP status code: {err}"), false))
    }

    fn read_chunk_size(stream: &mut Transport) -> io::Result<usize> {
        let mut line = Vec::with_capacity(16);
        loop {
            if line.len() >= 64 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "chunk size line too large",
                ));
            }
            let mut byte = [0u8; 1];
            let n = stream.read(&mut byte)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF while reading chunk size",
                ));
            }
            line.push(byte[0]);
            if line.ends_with(b"\r\n") {
                line.truncate(line.len() - 2);
                let hex = std::str::from_utf8(&line)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
                    .split_once(';')
                    .map_or_else(
                        || std::str::from_utf8(&line).unwrap_or(""),
                        |(size, _)| size,
                    )
                    .trim();
                return usize::from_str_radix(hex, 16)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err));
            }
        }
    }

    fn read_crlf(stream: &mut Transport) -> io::Result<()> {
        let mut crlf = [0u8; 2];
        stream.read_exact(&mut crlf)?;
        if crlf == *b"\r\n" {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid chunk terminator",
            ))
        }
    }

    fn drain_trailers(stream: &mut Transport) -> io::Result<()> {
        let mut total = 0usize;
        loop {
            let mut line = Vec::with_capacity(64);
            loop {
                if total >= HEADER_MAX_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "chunk trailers too large",
                    ));
                }
                let mut byte = [0u8; 1];
                let n = stream.read(&mut byte)?;
                if n == 0 {
                    return Ok(());
                }
                total += 1;
                line.push(byte[0]);
                if line.ends_with(b"\r\n") {
                    break;
                }
            }
            if line == b"\r\n" {
                return Ok(());
            }
            if line.len() >= HEADER_MAX_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "chunk trailers too large",
                ));
            }
        }
    }

    fn connect_with_timeout(
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<TcpStream, StreamError> {
        let mut last_err = None;
        let addrs = (host, port)
            .to_socket_addrs()
            .map_err(|err| StreamError::new(format!("resolve failed: {err}"), false))?;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => return Ok(stream),
                Err(err) => last_err = Some(err),
            }
        }
        Err(StreamError::new(
            format!(
                "connect failed: {}",
                last_err
                    .map(|err| err.to_string())
                    .unwrap_or_else(|| "no resolved address".to_string())
            ),
            false,
        ))
    }

    fn strip_ipv6_brackets(host: &str) -> &str {
        host.strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(host)
    }
}

#[cfg(all(feature = "winhttp", not(feature = "native-tls-client")))]
mod winhttp {
    use super::*;
    use std::{ffi::c_void, io};

    use windows::{
        Win32::{
            Foundation::GetLastError,
            Networking::WinHttp::{
                SECURITY_FLAG_IGNORE_CERT_CN_INVALID, SECURITY_FLAG_IGNORE_CERT_DATE_INVALID,
                SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE, SECURITY_FLAG_IGNORE_UNKNOWN_CA,
                WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_DISABLE_COOKIES, WINHTTP_FLAG_SECURE,
                WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_CONNECT_TIMEOUT,
                WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_OPTION_RECEIVE_TIMEOUT,
                WINHTTP_OPTION_RESOLVE_TIMEOUT, WINHTTP_OPTION_SECURITY_FLAGS,
                WINHTTP_OPTION_SEND_TIMEOUT, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
                WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
                WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
                WinHttpSetOption,
            },
        },
        core::PCWSTR,
    };

    const HTTPS_PORT: u16 = 443;
    const HTTP_PORT: u16 = 80;

    pub struct Session {
        handle: Handle,
    }

    impl Session {
        pub fn new(user_agent: &str) -> Result<Self, StreamError> {
            let handle = Handle::open_session(user_agent)?;
            handle.set_timeout(WINHTTP_OPTION_RESOLVE_TIMEOUT, 30_000)?;
            handle.set_timeout(WINHTTP_OPTION_CONNECT_TIMEOUT, 30_000)?;
            handle.set_timeout(WINHTTP_OPTION_SEND_TIMEOUT, 30_000)?;
            handle.set_timeout(WINHTTP_OPTION_RECEIVE_TIMEOUT, 90_000)?;
            Ok(Self { handle })
        }

        pub fn open_stream(
            &self,
            sub: &SubscriptionConfig,
            user_agent: &str,
            skip_tls_verify: bool,
        ) -> Result<HttpStream, StreamError> {
            let url = subscription_url(sub).map_err(|err| StreamError::new(err, false))?;
            let parsed =
                ParsedUrl::parse(&url).map_err(|err| StreamError::new(err.to_string(), false))?;
            let secure = parsed.scheme().eq_ignore_ascii_case("https");
            let (host, port) = parsed
                .host_port()
                .ok_or_else(|| StreamError::new("server URL is missing host", false))?;
            let port = port.unwrap_or(if secure { HTTPS_PORT } else { HTTP_PORT });
            let target = request_target(&parsed);
            let headers = request_headers(user_agent, sub);

            let connect = self.handle.connect(host, port)?;
            let request = connect.open_request(&target, secure)?;
            request.disable_features(WINHTTP_DISABLE_COOKIES)?;
            if secure && skip_tls_verify {
                request.set_security_flags(
                    SECURITY_FLAG_IGNORE_CERT_CN_INVALID
                        | SECURITY_FLAG_IGNORE_CERT_DATE_INVALID
                        | SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE
                        | SECURITY_FLAG_IGNORE_UNKNOWN_CA,
                )?;
            }
            request.send(&headers)?;
            request.receive_response()?;
            let status = request.status_code()?;
            if status != 200 {
                return Err(StreamError::new(
                    format!(
                        "HTTP {status} from {}",
                        crate::util::redact::redact_secret(&url)
                    ),
                    slow_status(status),
                ));
            }

            Ok(HttpStream::WinHttp(WinHttpStream {
                _connect: connect,
                request,
            }))
        }
    }

    pub struct WinHttpStream {
        _connect: Handle,
        request: Handle,
    }

    impl Read for WinHttpStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            let mut read = 0u32;
            unsafe {
                WinHttpReadData(
                    self.request.raw,
                    buf.as_mut_ptr().cast(),
                    buf.len().min(u32::MAX as usize) as u32,
                    &mut read,
                )
            }
            .map_err(io_error)?;
            Ok(read as usize)
        }
    }

    struct Handle {
        raw: *mut c_void,
    }

    impl Handle {
        fn open_session(user_agent: &str) -> Result<Self, StreamError> {
            let user_agent = wide(user_agent);
            let raw = unsafe {
                WinHttpOpen(
                    PCWSTR(user_agent.as_ptr()),
                    WINHTTP_ACCESS_TYPE_NO_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                )
            };
            Self::from_raw(raw, "WinHttpOpen failed")
        }

        fn connect(&self, host: &str, port: u16) -> Result<Self, StreamError> {
            let host = wide(strip_ipv6_brackets(host));
            let raw = unsafe { WinHttpConnect(self.raw, PCWSTR(host.as_ptr()), port, 0) };
            Self::from_raw(raw, "WinHttpConnect failed")
        }

        fn open_request(&self, target: &str, secure: bool) -> Result<Self, StreamError> {
            let verb = wide("GET");
            let target = wide(target);
            let flags = if secure {
                WINHTTP_FLAG_SECURE
            } else {
                WINHTTP_OPEN_REQUEST_FLAGS(0)
            };
            let raw = unsafe {
                WinHttpOpenRequest(
                    self.raw,
                    PCWSTR(verb.as_ptr()),
                    PCWSTR(target.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    flags,
                )
            };
            Self::from_raw(raw, "WinHttpOpenRequest failed")
        }

        fn send(&self, headers: &str) -> Result<(), StreamError> {
            let headers = wide_slice(headers);
            unsafe { WinHttpSendRequest(self.raw, Some(&headers), None, 0, 0, 0) }
                .map_err(|err| StreamError::new(format!("WinHttpSendRequest failed: {err}"), false))
        }

        fn receive_response(&self) -> Result<(), StreamError> {
            unsafe { WinHttpReceiveResponse(self.raw, std::ptr::null_mut()) }.map_err(|err| {
                StreamError::new(format!("WinHttpReceiveResponse failed: {err}"), false)
            })
        }

        fn status_code(&self) -> Result<u32, StreamError> {
            let mut status = 0u32;
            let mut len = std::mem::size_of::<u32>() as u32;
            let mut index = 0u32;
            unsafe {
                WinHttpQueryHeaders(
                    self.raw,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    PCWSTR::null(),
                    Some((&mut status as *mut u32).cast()),
                    &mut len,
                    &mut index,
                )
            }
            .map_err(|err| StreamError::new(format!("WinHttpQueryHeaders failed: {err}"), false))?;
            Ok(status)
        }

        fn set_timeout(&self, option: u32, millis: u32) -> Result<(), StreamError> {
            let bytes = millis.to_ne_bytes();
            unsafe { WinHttpSetOption(Some(self.raw.cast_const()), option, Some(&bytes)) }
                .map_err(|err| StreamError::new(format!("WinHttpSetOption failed: {err}"), false))
        }

        fn set_security_flags(&self, flags: u32) -> Result<(), StreamError> {
            let bytes = flags.to_ne_bytes();
            unsafe {
                WinHttpSetOption(
                    Some(self.raw.cast_const()),
                    WINHTTP_OPTION_SECURITY_FLAGS,
                    Some(&bytes),
                )
            }
            .map_err(|err| {
                StreamError::new(
                    format!("WinHttpSetOption security flags failed: {err}"),
                    false,
                )
            })
        }

        fn disable_features(&self, flags: u32) -> Result<(), StreamError> {
            let bytes = flags.to_ne_bytes();
            unsafe {
                WinHttpSetOption(
                    Some(self.raw.cast_const()),
                    WINHTTP_OPTION_DISABLE_FEATURE,
                    Some(&bytes),
                )
            }
            .map_err(|err| {
                StreamError::new(
                    format!("WinHttpSetOption disable features failed: {err}"),
                    false,
                )
            })
        }

        fn from_raw(raw: *mut c_void, message: &str) -> Result<Self, StreamError> {
            if raw.is_null() {
                Err(StreamError::new(
                    format!("{message}: {:?}", unsafe { GetLastError() }),
                    false,
                ))
            } else {
                Ok(Self { raw })
            }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.raw);
            }
        }
    }

    // WinHTTP permits concurrent creation of child handles from a shared
    // session handle; this wrapper closes the handle only through Drop.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}

    fn io_error(err: windows::core::Error) -> io::Error {
        io::Error::other(format!("WinHttpReadData failed: {err}"))
    }

    fn strip_ipv6_brackets(host: &str) -> &str {
        host.strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .unwrap_or(host)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn wide_slice(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
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
        time::Duration,
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
    fn drops_oversize_line_and_continues() {
        let input = br#"{"event":"keepalive","padding":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}
{"event":"message"}
"#;
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut items = Vec::new();
        let err = read_json_lines(&input[..], 64, &shutdown, |item| items.push(item)).unwrap_err();
        assert!(err.message.contains("stream ended"));
        assert!(matches!(items[0], StreamItem::InvalidJson(_)));
        assert!(matches!(items[1], StreamItem::Message(_)));
    }

    #[test]
    fn reports_invalid_then_oversize_line_without_closing_stream() {
        let input = b"{not-json}\n{\"event\":\"keepalive\",\"padding\":\"xxxxxxxx\"}\n{\"event\":\"message\"}\n";
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut items = Vec::new();
        let err = read_json_lines(&input[..], 32, &shutdown, |item| items.push(item)).unwrap_err();
        assert!(matches!(items[0], StreamItem::InvalidJson(_)));
        assert!(matches!(items[1], StreamItem::InvalidJson(_)));
        assert!(matches!(items[2], StreamItem::Message(_)));
        assert!(err.message.contains("stream ended"));
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
        let agent = agent(false).unwrap();
        let result = read_stream(&agent, &sub, "wintfy-rs/test", 1024, &shutdown, |item| {
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
        let agent = agent(false).unwrap();
        let err = read_stream(&agent, &sub, "wintfy-rs/test", 1024, &shutdown, |_| {}).unwrap_err();
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
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
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
