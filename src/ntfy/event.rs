use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NtfyEvent {
    pub id: Option<String>,
    pub time: Option<i64>,
    #[serde(default = "default_event")]
    pub event: String,
    pub topic: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    pub priority: Option<i32>,
    pub tags: Option<Vec<String>>,
    pub click: Option<String>,
    pub attachment: Option<NtfyAttachment>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NtfyAttachment {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub size: Option<u64>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: String,
    pub title: String,
    pub body: String,
    pub topic: String,
    pub priority: i32,
    pub click_url: Option<String>,
}

fn default_event() -> String {
    "message".to_string()
}

pub fn parse_line(line: &str) -> Result<NtfyEvent, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_message_event() {
        let event = parse_line(
            r#"{"id":"x","time":1,"event":"message","topic":"alerts","message":"hello","priority":3}"#,
        )
        .unwrap();
        assert_eq!(event.topic.as_deref(), Some("alerts"));
        assert_eq!(event.priority, Some(3));
    }

    #[test]
    fn ignores_unknown_fields() {
        let event = parse_line(r#"{"event":"keepalive","unknown":true}"#).unwrap();
        assert_eq!(event.event, "keepalive");
    }
}
