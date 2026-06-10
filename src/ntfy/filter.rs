use crate::{
    config::{NotificationConfig, SecurityConfig},
    ntfy::event::{Notification, NtfyAttachment, NtfyEvent},
    util::{lru::LruIds, url::ParsedUrl},
};

const SAFE_CLICK_URL_SCHEMES: &[&str] = &["http", "https"];

pub struct MessageFilter {
    ids: LruIds,
    notification: NotificationConfig,
    security: SecurityConfig,
}

impl MessageFilter {
    pub fn new(notification: NotificationConfig, security: SecurityConfig) -> Self {
        Self {
            ids: LruIds::new(256),
            notification,
            security,
        }
    }

    pub fn filter(&mut self, event: NtfyEvent) -> Option<Notification> {
        let priority = event.priority.unwrap_or(3);
        if priority < self.notification.min_priority {
            return None;
        }
        if let Some(id) = event.id.as_deref()
            && !self.ids.insert_new(id)
        {
            return None;
        }

        let topic = event.topic.unwrap_or_else(|| "ntfy".to_string());
        let mut title = event
            .title
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| topic.clone());
        if self.notification.tags_as_emoji_prefix
            && let Some(tags) = event.tags.as_ref()
        {
            let mut prefix = String::new();
            for tag in tags {
                if let Some(value) = tag_to_prefix(tag) {
                    prefix.push_str(value);
                }
            }
            if !prefix.is_empty() {
                prefix.push(' ');
                prefix.push_str(&title);
                title = prefix;
            }
        }
        title = truncate(&title, self.notification.max_title_len);

        let mut body = event.message.unwrap_or_default();
        if let Some(attachment) = event.attachment.as_ref() {
            let attachment_line = attachment_summary(attachment);
            if !attachment_line.is_empty() {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&attachment_line);
            }
        }
        let click_url = valid_click_url(
            event.click.as_deref(),
            &self.security.allow_url_schemes,
            self.security.allow_dangerous_url_schemes,
            self.security.max_click_url_len,
        );
        if body.trim().is_empty() && click_url.is_none() {
            return None;
        }
        use std::fmt::Write as _;
        let _ = write!(body, "\nTopic: {topic}\nPriority: {priority}");
        body = truncate(&body, self.notification.max_body_len);

        Some(Notification {
            id: event.id.unwrap_or_default(),
            title,
            body,
            topic,
            priority,
            click_url,
        })
    }
}

fn attachment_summary(attachment: &NtfyAttachment) -> String {
    let Some(name) = attachment.name.as_deref() else {
        return String::new();
    };
    match attachment.size {
        Some(size) => format!("Attachment: {name} ({})", format_size(size)),
        None => format!("Attachment: {name}"),
    }
}

fn format_size(size: u64) -> String {
    if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / 1024.0 / 1024.0)
    } else if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

fn tag_to_prefix(tag: &str) -> Option<&'static str> {
    match tag.to_ascii_lowercase().as_str() {
        "warning" | "warn" => Some("⚠️"),
        "error" | "alert" | "rotating_light" => Some("🚨"),
        "white_check_mark" | "ok" | "success" => Some("✅"),
        "x" | "failed" | "failure" => Some("❌"),
        "info" => Some("ℹ️"),
        _ => None,
    }
}

pub fn truncate(input: &str, max_chars: usize) -> String {
    let keep = max_chars.saturating_sub(1);
    let mut chars = input.char_indices();
    for _ in 0..max_chars {
        if chars.next().is_none() {
            return input.to_string();
        }
    }
    let byte_end = if keep == 0 {
        0
    } else {
        input
            .char_indices()
            .nth(keep)
            .map(|(idx, _)| idx)
            .unwrap_or(input.len())
    };
    let mut output = String::with_capacity(byte_end + '…'.len_utf8());
    output.push_str(&input[..byte_end]);
    output.push('…');
    output
}

pub fn valid_click_url(
    input: Option<&str>,
    allowed_schemes: &[String],
    allow_dangerous_schemes: bool,
    max_len: usize,
) -> Option<String> {
    let input = input?;
    if input.len() > max_len {
        log::warn!("click URL too long, ignoring");
        return None;
    }
    let url = ParsedUrl::parse(input).ok()?;
    let scheme = url.scheme();
    if !allowed_schemes
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(scheme))
    {
        log::warn!("click URL scheme rejected by allowlist: {scheme}");
        return None;
    }
    if SAFE_CLICK_URL_SCHEMES
        .iter()
        .any(|safe| safe.eq_ignore_ascii_case(scheme))
    {
        return Some(input.to_string());
    }
    if allow_dangerous_schemes {
        log::warn!("dangerous click URL scheme allowed by config: {scheme}");
        Some(input.to_string())
    } else {
        log::warn!("dangerous click URL scheme rejected: {scheme}");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_priority() {
        let mut filter = MessageFilter::new(
            NotificationConfig {
                min_priority: 4,
                ..Default::default()
            },
            SecurityConfig::default(),
        );
        let event = NtfyEvent {
            event: "message".to_string(),
            topic: Some("a".to_string()),
            message: Some("hello".to_string()),
            priority: Some(3),
            id: Some("1".to_string()),
            time: None,
            title: None,
            tags: None,
            click: None,
            attachment: None,
        };
        assert!(filter.filter(event).is_none());
    }

    #[test]
    fn suppresses_duplicates() {
        let mut filter =
            MessageFilter::new(NotificationConfig::default(), SecurityConfig::default());
        let event = NtfyEvent {
            event: "message".to_string(),
            topic: Some("a".to_string()),
            message: Some("hello".to_string()),
            priority: Some(3),
            id: Some("1".to_string()),
            time: None,
            title: None,
            tags: None,
            click: None,
            attachment: None,
        };
        assert!(filter.filter(event.clone()).is_some());
        assert!(filter.filter(event).is_none());
    }

    #[test]
    fn rejects_non_http_click() {
        assert!(
            valid_click_url(
                Some("file:///C:/x"),
                &["http".to_string(), "https".to_string()],
                false,
                2048
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_dangerous_click_even_when_allowlisted_without_opt_in() {
        assert!(
            valid_click_url(
                Some("ms-settings:privacy"),
                &[
                    "http".to_string(),
                    "https".to_string(),
                    "ms-settings".to_string()
                ],
                false,
                2048
            )
            .is_none()
        );
    }

    #[test]
    fn allows_dangerous_click_only_when_allowlisted_and_opted_in() {
        assert_eq!(
            valid_click_url(
                Some("file:///C:/x"),
                &["http".to_string(), "https".to_string(), "file".to_string()],
                true,
                2048
            )
            .as_deref(),
            Some("file:///C:/x")
        );
    }
}
