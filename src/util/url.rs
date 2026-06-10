#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl<'a> {
    original: &'a str,
    scheme: &'a str,
    authority: Option<&'a str>,
    path_start: usize,
    query_start: Option<usize>,
    fragment_start: Option<usize>,
}

impl<'a> ParsedUrl<'a> {
    pub fn parse(input: &'a str) -> Result<Self, UrlError> {
        if input.is_empty() || has_control_or_space(input) {
            return Err(UrlError::Invalid);
        }
        let Some(colon) = input.find(':') else {
            return Err(UrlError::MissingScheme);
        };
        let scheme = &input[..colon];
        if !valid_scheme(scheme) {
            return Err(UrlError::InvalidScheme);
        }

        let rest = &input[colon + 1..];
        let (authority, path_start) = if let Some(after_slashes) = rest.strip_prefix("//") {
            let authority_start = colon + 3;
            let authority_len = after_slashes
                .find(['/', '?', '#'])
                .unwrap_or(after_slashes.len());
            let authority = &input[authority_start..authority_start + authority_len];
            if !authority.is_empty() && !valid_authority(authority) {
                return Err(UrlError::InvalidAuthority);
            }
            (Some(authority), authority_start + authority_len)
        } else {
            (None, colon + 1)
        };

        let query_start = input[path_start..].find('?').map(|pos| path_start + pos);
        let fragment_start = input[path_start..].find('#').map(|pos| path_start + pos);
        Ok(Self {
            original: input,
            scheme,
            authority,
            path_start,
            query_start,
            fragment_start,
        })
    }

    pub fn scheme(&self) -> &str {
        self.scheme
    }

    #[cfg(feature = "native-tls-client")]
    pub fn authority(&self) -> Option<&str> {
        self.authority
    }

    #[cfg(any(feature = "winhttp", feature = "native-tls-client", test))]
    pub fn host_port(&self) -> Option<(&str, Option<u16>)> {
        let authority = self.authority?;
        split_host_port(authority)
    }

    pub fn path(&self) -> &str {
        let end = self
            .query_start
            .or(self.fragment_start)
            .unwrap_or(self.original.len());
        &self.original[self.path_start..end]
    }

    pub fn with_redacted_query(&self) -> Option<String> {
        let query_start = self.query_start?;
        let fragment = self.fragment_start.map(|start| &self.original[start..]);
        let end = self.fragment_start.unwrap_or(self.original.len());
        if query_start >= end {
            return None;
        }
        let mut output = String::with_capacity(query_start + "?<redacted>".len());
        output.push_str(&self.original[..query_start]);
        output.push_str("?<redacted>");
        if let Some(fragment) = fragment {
            output.push_str(fragment);
        }
        Some(output)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    Invalid,
    MissingScheme,
    InvalidScheme,
    MissingHost,
    InvalidAuthority,
    UnsupportedScheme,
}

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UrlError::Invalid => f.write_str("invalid URL"),
            UrlError::MissingScheme => f.write_str("missing URL scheme"),
            UrlError::InvalidScheme => f.write_str("invalid URL scheme"),
            UrlError::MissingHost => f.write_str("missing URL host"),
            UrlError::InvalidAuthority => f.write_str("invalid URL authority"),
            UrlError::UnsupportedScheme => f.write_str("unsupported URL scheme"),
        }
    }
}

pub fn build_stream_url(server: &str, topics: &[String]) -> Result<String, UrlError> {
    let parsed = ParsedUrl::parse(server)?;
    let Some(authority) = parsed.authority.filter(|authority| !authority.is_empty()) else {
        return Err(UrlError::MissingHost);
    };
    let path = parsed.path().trim_end_matches('/');
    let topics_len = topics
        .iter()
        .map(|topic| topic.trim().len())
        .sum::<usize>()
        .saturating_add(topics.len().saturating_sub(1));

    let mut output = String::with_capacity(
        parsed.scheme.len()
            + "://".len()
            + authority.len()
            + path.len()
            + 1
            + topics_len
            + "/json".len(),
    );
    output.push_str(parsed.scheme);
    output.push_str("://");
    output.push_str(authority);
    output.push_str(path);
    output.push('/');
    for topic in topics
        .iter()
        .map(|topic| topic.trim())
        .filter(|topic| !topic.is_empty())
    {
        if !output.ends_with('/') {
            output.push(',');
        }
        output.push_str(topic);
    }
    output.push_str("/json");
    Ok(output)
}

pub fn validate_http_server(server: &str, allow_http: bool) -> Result<&str, UrlError> {
    let parsed = ParsedUrl::parse(server)?;
    if parsed
        .authority
        .is_none_or(|authority| authority.is_empty() || split_host_port(authority).is_none())
    {
        return Err(UrlError::MissingHost);
    }
    match parsed.scheme {
        scheme if matches_ignore_ascii_case(scheme, "https") => Ok("https"),
        scheme if matches_ignore_ascii_case(scheme, "http") && allow_http => Ok("http"),
        scheme if matches_ignore_ascii_case(scheme, "http") => Err(UrlError::UnsupportedScheme),
        _ => Err(UrlError::UnsupportedScheme),
    }
}

pub fn has_control_or_space(input: &str) -> bool {
    input.bytes().any(|byte| byte <= b' ' || byte == 0x7f)
}

pub fn valid_topic(topic: &str) -> bool {
    !topic.is_empty()
        && !has_control_or_space(topic)
        && !topic
            .bytes()
            .any(|byte| matches!(byte, b'/' | b',' | b'?' | b'#'))
}

pub fn matches_ignore_ascii_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn valid_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

fn valid_authority(authority: &str) -> bool {
    !authority
        .bytes()
        .any(|byte| matches!(byte, b'/' | b'?' | b'#'))
}

fn split_host_port(authority: &str) -> Option<(&str, Option<u16>)> {
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = &authority[..end + 2];
        let after = &authority[end + 2..];
        if after.is_empty() {
            return Some((host, None));
        }
        let port = after.strip_prefix(':')?.parse().ok()?;
        return Some((host, Some(port)));
    }

    let mut parts = authority.rsplitn(2, ':');
    let last = parts.next()?;
    let before = parts.next();
    match before {
        Some(host) if !host.contains(':') && !last.is_empty() => {
            Some((host, Some(last.parse().ok()?)))
        }
        Some(_) => Some((authority, None)),
        None => Some((authority, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_url_parts() {
        let parsed = ParsedUrl::parse("https://example.com/ntfy?a=1#frag").unwrap();
        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_port(), Some(("example.com", None)));
        assert_eq!(parsed.path(), "/ntfy");
        assert!(parsed.with_redacted_query().is_some());
    }

    #[test]
    fn builds_stream_url_with_prefix_and_ipv6() {
        assert_eq!(
            build_stream_url(
                "https://[::1]:8080/ntfy/?token=secret",
                &["a".to_string(), "b".to_string()]
            )
            .unwrap(),
            "https://[::1]:8080/ntfy/a,b/json"
        );
    }

    #[test]
    fn splits_host_port() {
        assert_eq!(
            ParsedUrl::parse("https://example.com:8443/x")
                .unwrap()
                .host_port(),
            Some(("example.com", Some(8443)))
        );
        assert_eq!(
            ParsedUrl::parse("https://[::1]:8443/x")
                .unwrap()
                .host_port(),
            Some(("[::1]", Some(8443)))
        );
        assert_eq!(
            ParsedUrl::parse("https://example.com/x")
                .unwrap()
                .host_port(),
            Some(("example.com", None))
        );
    }

    #[test]
    fn rejects_missing_or_bad_authority() {
        assert!(validate_http_server("https:///x", false).is_err());
        assert!(ParsedUrl::parse("https://exa mple.com").is_err());
        assert!(validate_http_server("file:///x", false).is_err());
        assert!(validate_http_server("https://user:pass@example.com", false).is_err());
    }

    #[test]
    fn rejects_unsafe_topics() {
        assert!(valid_topic("alerts"));
        assert!(!valid_topic("a/b"));
        assert!(!valid_topic("a,b"));
        assert!(!valid_topic("a b"));
        assert!(!valid_topic("a?b"));
    }

    #[test]
    fn redacts_query_without_losing_fragment() {
        let parsed = ParsedUrl::parse("https://x.test/a?token=abc#frag").unwrap();
        assert_eq!(
            parsed.with_redacted_query().as_deref(),
            Some("https://x.test/a?<redacted>#frag")
        );
    }
}
