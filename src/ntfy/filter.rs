use std::sync::Arc;

use crate::{
    config::{NotificationConfig, SecurityConfig},
    ntfy::event::{Notification, NtfyAttachment, NtfyEvent},
    util::{lru::LruIds, url::ParsedUrl},
};

pub struct MessageFilter {
    ids: LruIds,
    notification: Arc<NotificationConfig>,
    security: Arc<SecurityConfig>,
}

impl MessageFilter {
    #[cfg(test)]
    pub fn new(notification: NotificationConfig, security: SecurityConfig) -> Self {
        Self::from_shared(Arc::new(notification), Arc::new(security))
    }

    pub fn from_shared(
        notification: Arc<NotificationConfig>,
        security: Arc<SecurityConfig>,
    ) -> Self {
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

        let tags_len_hint = event
            .tags
            .as_ref()
            .map(|tags| tags.len().saturating_mul(5))
            .unwrap_or(0);
        let topic = event.topic.unwrap_or_else(|| "ntfy".to_string());
        let raw_title = event
            .title
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&topic);
        let mut title = String::with_capacity(raw_title.len().saturating_add(tags_len_hint));
        if self.notification.tags_as_emoji_prefix
            && let Some(tags) = event.tags.as_ref()
        {
            for tag in tags {
                if let Some(value) = tag_to_prefix(tag) {
                    title.push_str(value);
                }
            }
            if !title.is_empty() {
                title.push(' ');
            }
        }
        title.push_str(raw_title);
        let title = truncate_owned(title, self.notification.max_title_len);

        let mut body = event.message.unwrap_or_default();
        if let Some(attachment) = event.attachment.as_ref()
            && attachment.name.is_some()
        {
            if !body.is_empty() {
                body.push('\n');
            }
            append_attachment_summary(&mut body, attachment);
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
        let body = truncate_owned(body, self.notification.max_body_len);

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

fn append_attachment_summary(output: &mut String, attachment: &NtfyAttachment) {
    let Some(name) = attachment.name.as_deref() else {
        return;
    };
    output.push_str("Attachment: ");
    output.push_str(name);
    if let Some(size) = attachment.size {
        output.push_str(" (");
        write_size(output, size);
        output.push(')');
    }
}

fn write_size(output: &mut String, size: u64) {
    use std::fmt::Write as _;
    if size >= 1024 * 1024 {
        write_one_decimal(output, size, 1024 * 1024, " MB");
    } else if size >= 1024 {
        write_one_decimal(output, size, 1024, " KB");
    } else {
        let _ = write!(output, "{size} B");
    }
}

fn write_one_decimal(output: &mut String, size: u64, unit: u64, suffix: &str) {
    use std::fmt::Write as _;
    let scaled = size.saturating_mul(10).saturating_add(unit / 2) / unit;
    let _ = write!(output, "{}.{}{}", scaled / 10, scaled % 10, suffix);
}

fn tag_to_prefix(tag: &str) -> Option<&'static str> {
    if tag.eq_ignore_ascii_case("warning") || tag.eq_ignore_ascii_case("warn") {
        Some("⚠️")
    } else if tag.eq_ignore_ascii_case("error")
        || tag.eq_ignore_ascii_case("alert")
        || tag.eq_ignore_ascii_case("rotating_light")
    {
        Some("🚨")
    } else if tag.eq_ignore_ascii_case("white_check_mark")
        || tag.eq_ignore_ascii_case("ok")
        || tag.eq_ignore_ascii_case("success")
    {
        Some("✅")
    } else if tag.eq_ignore_ascii_case("x")
        || tag.eq_ignore_ascii_case("failed")
        || tag.eq_ignore_ascii_case("failure")
    {
        Some("❌")
    } else if tag.eq_ignore_ascii_case("info") {
        Some("ℹ️")
    } else {
        None
    }
}

#[cfg(test)]
fn truncate(input: &str, max_chars: usize) -> String {
    truncate_owned(input.to_string(), max_chars)
}

fn truncate_owned(mut input: String, max_chars: usize) -> String {
    let keep = max_chars.saturating_sub(1);
    let mut chars = input.char_indices();
    for _ in 0..max_chars {
        if chars.next().is_none() {
            return input;
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
    input.truncate(byte_end);
    input.push('…');
    input
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
    if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
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

    #[test]
    fn truncates_without_splitting_unicode() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("警报测试", 3), "警报…");
    }

    #[test]
    fn formats_attachment_size_like_decimal_display() {
        let mut output = String::new();
        write_size(&mut output, 1536);
        assert_eq!(output, "1.5 KB");

        output.clear();
        write_size(&mut output, 1024 * 1024 + 512 * 1024);
        assert_eq!(output, "1.5 MB");
    }

    #[test]
    fn tag_prefix_matches_ascii_case_insensitively() {
        assert_eq!(tag_to_prefix("WARNING"), Some("⚠️"));
        assert_eq!(tag_to_prefix("Rotating_Light"), Some("🚨"));
        assert_eq!(tag_to_prefix("White_Check_Mark"), Some("✅"));
    }
}
