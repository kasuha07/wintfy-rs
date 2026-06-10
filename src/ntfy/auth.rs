use base64::{Engine, engine::general_purpose::STANDARD};

use crate::config::{AuthKind, SubscriptionConfig};

pub fn auth_header(sub: &SubscriptionConfig) -> Option<String> {
    match sub.auth {
        AuthKind::None => None,
        AuthKind::Bearer => sub.token.as_ref().map(|token| format!("Bearer {token}")),
        AuthKind::Basic => {
            let username = sub.username.as_deref()?;
            let password = sub.password.as_deref()?;
            Some(format!(
                "Basic {}",
                STANDARD.encode(format!("{username}:{password}"))
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthKind;

    #[test]
    fn builds_bearer_header() {
        let mut sub = SubscriptionConfig::default();
        sub.auth = AuthKind::Bearer;
        sub.token = Some("abc".to_string());
        assert_eq!(auth_header(&sub).as_deref(), Some("Bearer abc"));
    }

    #[test]
    fn builds_basic_header() {
        let mut sub = SubscriptionConfig::default();
        sub.auth = AuthKind::Basic;
        sub.username = Some("user".to_string());
        sub.password = Some("pass".to_string());
        assert_eq!(auth_header(&sub).as_deref(), Some("Basic dXNlcjpwYXNz"));
    }
}
