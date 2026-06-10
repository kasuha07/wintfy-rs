use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::platform::paths;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub app: AppConfig,
    pub notification: NotificationConfig,
    pub network: NetworkConfig,
    pub security: SecurityConfig,
    pub subscriptions: Vec<SubscriptionConfig>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    pub log_level: String,
    pub show_connection_status_toast: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NotificationConfig {
    pub min_priority: i32,
    pub tags_as_emoji_prefix: bool,
    pub max_title_len: usize,
    pub max_body_len: usize,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NetworkConfig {
    pub reconnect_initial_seconds: u64,
    pub reconnect_max_seconds: u64,
    pub reconnect_jitter: bool,
    pub line_max_bytes: usize,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SecurityConfig {
    pub allow_url_schemes: Vec<String>,
    pub allow_dangerous_url_schemes: bool,
    pub max_click_url_len: usize,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SubscriptionConfig {
    pub name: String,
    pub server: String,
    pub topics: Vec<String>,
    pub auth: AuthKind,
    pub token: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    #[default]
    None,
    Bearer,
    Basic,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: Config,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app: AppConfig::default(),
            notification: NotificationConfig::default(),
            network: NetworkConfig::default(),
            security: SecurityConfig::default(),
            subscriptions: vec![SubscriptionConfig {
                name: "default".to_string(),
                server: "https://ntfy.sh".to_string(),
                topics: vec!["wintfy-rs".to_string()],
                auth: AuthKind::None,
                token: None,
                username: None,
                password: None,
            }],
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            show_connection_status_toast: false,
        }
    }
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            min_priority: 1,
            tags_as_emoji_prefix: true,
            max_title_len: 96,
            max_body_len: 512,
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            reconnect_initial_seconds: 2,
            reconnect_max_seconds: 60,
            reconnect_jitter: true,
            line_max_bytes: 1024 * 1024,
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allow_url_schemes: vec!["http".to_string(), "https".to_string()],
            allow_dangerous_url_schemes: false,
            max_click_url_len: 2048,
        }
    }
}

impl Default for SubscriptionConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            server: String::new(),
            topics: Vec::new(),
            auth: AuthKind::None,
            token: None,
            username: None,
            password: None,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        parse_level(&self.app.log_level)
            .ok_or_else(|| format!("invalid log_level: {}", self.app.log_level))?;
        if self.notification.max_title_len == 0 {
            return Err("notification.max_title_len must be greater than 0".to_string());
        }
        if self.notification.max_body_len == 0 {
            return Err("notification.max_body_len must be greater than 0".to_string());
        }
        if self.network.reconnect_initial_seconds == 0 {
            return Err("network.reconnect_initial_seconds must be greater than 0".to_string());
        }
        if self.network.reconnect_max_seconds < self.network.reconnect_initial_seconds {
            return Err(
                "network.reconnect_max_seconds must be >= reconnect_initial_seconds".to_string(),
            );
        }
        if self.network.line_max_bytes < 1024 {
            return Err("network.line_max_bytes must be at least 1024".to_string());
        }
        if self.security.max_click_url_len < 16 {
            return Err("security.max_click_url_len must be at least 16".to_string());
        }
        if self.subscriptions.is_empty() {
            return Err("at least one [[subscriptions]] entry is required".to_string());
        }

        for sub in &self.subscriptions {
            validate_subscription(sub)?;
        }
        Ok(())
    }

    pub fn example_toml() -> &'static str {
        r#"[app]
log_level = "info"

[notification]
min_priority = 1
tags_as_emoji_prefix = true
max_title_len = 96
max_body_len = 512

[network]
reconnect_initial_seconds = 2
reconnect_max_seconds = 60
reconnect_jitter = true
line_max_bytes = 1048576

[security]
allow_url_schemes = ["http", "https"]
allow_dangerous_url_schemes = false
max_click_url_len = 2048

[[subscriptions]]
name = "default"
server = "https://ntfy.sh"
topics = ["wintfy-rs"]
"#
    }
}

fn validate_subscription(sub: &SubscriptionConfig) -> Result<(), String> {
    if sub.name.trim().is_empty() {
        return Err("subscription.name is required".to_string());
    }
    if sub.server.trim().is_empty() {
        return Err(format!("subscription {} server is required", sub.name));
    }
    let parsed = url::Url::parse(&sub.server)
        .map_err(|err| format!("subscription {} server URL is invalid: {err}", sub.name))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "subscription {} unsupported server scheme: {other}",
                sub.name
            ));
        }
    }
    if sub.topics.is_empty() {
        return Err(format!(
            "subscription {} must include at least one topic",
            sub.name
        ));
    }
    for topic in &sub.topics {
        if topic.trim().is_empty() || topic.contains('/') || topic.contains(',') {
            return Err(format!(
                "subscription {} has invalid topic: {topic}",
                sub.name
            ));
        }
    }
    match sub.auth {
        AuthKind::None => {}
        AuthKind::Bearer => {
            let token = sub.token.as_deref().unwrap_or("");
            if token.trim().is_empty() {
                return Err(format!(
                    "subscription {} bearer auth requires token",
                    sub.name
                ));
            }
            if token.chars().any(char::is_control) {
                return Err(format!(
                    "subscription {} bearer token must not contain control characters",
                    sub.name
                ));
            }
        }
        AuthKind::Basic => {
            if sub.username.as_deref().unwrap_or("").is_empty()
                || sub.password.as_deref().unwrap_or("").is_empty()
            {
                return Err(format!(
                    "subscription {} basic auth requires username and password",
                    sub.name
                ));
            }
        }
    }
    Ok(())
}

pub fn load_config(explicit: Option<&Path>) -> Result<LoadedConfig, String> {
    let path = resolve_config_path(explicit)?;
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read config {}: {err}", path.display()))?;
    let config: Config = toml::from_str(&text)
        .map_err(|err| format!("failed to parse config {}: {err}", path.display()))?;
    config.validate()?;
    Ok(LoadedConfig { path, config })
}

pub fn resolve_config_path(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let portable = exe_dir.join("config.toml");
    if portable.exists() {
        return Ok(portable);
    }
    Ok(paths::config_file()?)
}

pub fn ensure_default_config(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create config directory {}: {err}",
                parent.display()
            )
        })?;
    }
    fs::write(path, Config::example_toml())
        .map_err(|err| format!("failed to write default config {}: {err}", path.display()))
}

pub fn parse_level(level: &str) -> Option<log::LevelFilter> {
    match level.to_ascii_lowercase().as_str() {
        "error" => Some(log::LevelFilter::Error),
        "warn" | "warning" => Some(log::LevelFilter::Warn),
        "info" => Some(log::LevelFilter::Info),
        "debug" => Some(log::LevelFilter::Debug),
        "trace" => Some(log::LevelFilter::Trace),
        "off" => Some(log::LevelFilter::Off),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_config() {
        let config: Config = toml::from_str(Config::example_toml()).unwrap();
        config.validate().unwrap();
        assert_eq!(config.subscriptions[0].server, "https://ntfy.sh");
    }

    #[test]
    fn rejects_missing_bearer_token() {
        let mut config = Config::default();
        config.subscriptions[0].auth = AuthKind::Bearer;
        config.subscriptions[0].token = None;
        assert!(config.validate().unwrap_err().contains("token"));
    }

    #[test]
    fn rejects_bearer_token_with_crlf() {
        let mut config = Config::default();
        config.subscriptions[0].auth = AuthKind::Bearer;
        config.subscriptions[0].token = Some("abc\r\nX-Injected: yes".to_string());
        assert!(config.validate().unwrap_err().contains("control"));
    }

    #[test]
    fn rejects_bearer_token_with_other_control_chars() {
        let mut config = Config::default();
        config.subscriptions[0].auth = AuthKind::Bearer;
        config.subscriptions[0].token = Some("abc\u{7f}".to_string());
        assert!(config.validate().unwrap_err().contains("control"));
    }

    #[test]
    fn rejects_bad_topic() {
        let mut config = Config::default();
        config.subscriptions[0].topics = vec!["a/b".to_string()];
        assert!(config.validate().is_err());
    }
}
