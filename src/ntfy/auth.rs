use crate::config::{AuthKind, SubscriptionConfig};

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn auth_header(sub: &SubscriptionConfig) -> Option<String> {
    match sub.auth {
        AuthKind::None => None,
        AuthKind::Bearer => sub.token.as_ref().map(|token| format!("Bearer {token}")),
        AuthKind::Basic => {
            let username = sub.username.as_deref()?;
            let password = sub.password.as_deref()?;
            Some(format!("Basic {}", basic_token(username, password)))
        }
    }
}

fn basic_token(username: &str, password: &str) -> String {
    let mut input = String::with_capacity(username.len() + 1 + password.len());
    input.push_str(username);
    input.push(':');
    input.push_str(password);
    encode_base64(input.as_bytes())
}

fn encode_base64(input: &[u8]) -> String {
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        output.push(BASE64[(b0 >> 2) as usize] as char);
        output.push(BASE64[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(BASE64[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(BASE64[(b2 & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthKind;

    #[test]
    fn builds_bearer_header() {
        let sub = SubscriptionConfig {
            auth: AuthKind::Bearer,
            token: Some("abc".to_string()),
            ..Default::default()
        };
        assert_eq!(auth_header(&sub).as_deref(), Some("Bearer abc"));
    }

    #[test]
    fn builds_basic_header() {
        let sub = SubscriptionConfig {
            auth: AuthKind::Basic,
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            ..Default::default()
        };
        assert_eq!(auth_header(&sub).as_deref(), Some("Basic dXNlcjpwYXNz"));
    }

    #[test]
    fn base64_padding_cases() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
    }
}
