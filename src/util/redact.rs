pub fn redact_secret(input: &str) -> String {
    let mut output = input.to_string();
    for key in ["token", "password", "authorization", "auth"] {
        output = redact_key_values(&output, key);
    }
    redact_url_query(&output)
}

fn redact_key_values(input: &str, key: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut result = String::with_capacity(input.len());
    let mut index = 0;

    while let Some(pos) = lower[index..].find(key) {
        let start = index + pos;
        result.push_str(&input[index..start + key.len()]);
        let after = start + key.len();
        let bytes = input.as_bytes();
        let mut cursor = after;
        while cursor < input.len() && bytes[cursor].is_ascii_whitespace() {
            result.push(bytes[cursor] as char);
            cursor += 1;
        }
        if cursor < input.len() && matches!(bytes[cursor], b'=' | b':') {
            result.push(bytes[cursor] as char);
            cursor += 1;
            while cursor < input.len() && bytes[cursor].is_ascii_whitespace() {
                result.push(bytes[cursor] as char);
                cursor += 1;
            }
            result.push_str("<redacted>");
            while cursor < input.len() {
                if matches!(bytes[cursor], b',' | b';' | b'\r' | b'\n') {
                    break;
                }
                if key != "authorization" && bytes[cursor].is_ascii_whitespace() {
                    break;
                }
                cursor += 1;
            }
            index = cursor;
        } else {
            index = after;
        }
    }
    result.push_str(&input[index..]);
    result
}

pub fn redact_url_query(input: &str) -> String {
    let mut result = Vec::new();
    for part in input.split_whitespace() {
        if let Ok(url) = crate::util::url::ParsedUrl::parse(part) {
            result.push(
                url.with_redacted_query()
                    .unwrap_or_else(|| part.to_string()),
            );
        } else {
            result.push(part.to_string());
        }
    }
    result.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_key_value_pairs() {
        let text = redact_secret("token = abc password: def Authorization: Basic aaa");
        assert!(!text.contains("abc"));
        assert!(!text.contains("def"));
        assert!(!text.contains("aaa"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn redacts_url_query() {
        let text = redact_secret("failed https://x.test/topic?token=abc&x=1");
        assert!(!text.contains("abc"));
        assert!(text.contains("%3Credacted%3E") || text.contains("<redacted>"));
    }
}
