use crate::{ntfy::event::Notification, platform::paths};

use windows::{Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID, core::PCWSTR};

pub fn init_app_id() -> Result<(), String> {
    let id = wide(paths::AUMID);
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(PCWSTR(id.as_ptr()))
            .map_err(|err| format!("SetCurrentProcessExplicitAppUserModelID failed: {err}"))?;
    }
    Ok(())
}

#[cfg(feature = "native-toast")]
pub fn show(notification: &Notification) -> Result<(), String> {
    let xml = toast_xml(
        &notification.title,
        &notification.body,
        notification.click_url.as_deref(),
    );
    show_xml(&xml)
}

#[cfg(not(feature = "native-toast"))]
pub fn show(_notification: &Notification) -> Result<(), String> {
    Err("native toast support is disabled at build time".to_string())
}

#[cfg(feature = "native-toast")]
pub fn show_test() -> Result<(), String> {
    show_xml(&toast_xml("wintfy-rs", "Test notification", None))
}

#[cfg(not(feature = "native-toast"))]
pub fn show_test() -> Result<(), String> {
    Err("native toast support is disabled at build time".to_string())
}

#[cfg(feature = "native-toast")]
pub fn show_error(title: &str, body: &str) -> Result<(), String> {
    show_xml(&toast_xml(title, body, None))
}

#[cfg(not(feature = "native-toast"))]
pub fn show_error(_title: &str, _body: &str) -> Result<(), String> {
    Err("native toast support is disabled at build time".to_string())
}

#[cfg(feature = "native-toast")]
fn show_xml(xml: &str) -> Result<(), String> {
    use windows::{
        Data::Xml::Dom::XmlDocument,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
        core::HSTRING,
    };

    let doc = XmlDocument::new().map_err(|err| format!("XmlDocument creation failed: {err}"))?;
    doc.LoadXml(&HSTRING::from(xml))
        .map_err(|err| format!("Toast XML parse failed: {err}"))?;
    let toast = ToastNotification::CreateToastNotification(&doc)
        .map_err(|err| format!("ToastNotification creation failed: {err}"))?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(paths::AUMID))
            .map_err(|err| format!("Toast notifier creation failed: {err}"))?;
    notifier
        .Show(&toast)
        .map_err(|err| format!("Toast show failed: {err}"))?;
    Ok(())
}

#[cfg(feature = "native-toast")]
fn toast_xml(title: &str, body: &str, launch: Option<&str>) -> String {
    let mut xml = String::with_capacity(
        title
            .len()
            .saturating_add(body.len())
            .saturating_add(launch.map(str::len).unwrap_or(0))
            .saturating_add(128),
    );
    xml.push_str("<toast");
    if let Some(url) = launch {
        xml.push_str(r#" activationType="protocol" launch=""#);
        push_xml_attr(&mut xml, url);
        xml.push('"');
    }
    xml.push_str(r#"><visual><binding template="ToastGeneric"><text>"#);
    push_xml_text(&mut xml, title);
    xml.push_str("</text><text>");
    push_xml_text(&mut xml, body);
    xml.push_str("</text></binding></visual></toast>");
    xml
}

#[cfg(feature = "native-toast")]
fn push_xml_text(output: &mut String, input: &str) {
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(ch),
        }
    }
}

#[cfg(feature = "native-toast")]
fn push_xml_attr(output: &mut String, input: &str) {
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(ch),
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(all(test, feature = "native-toast"))]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml() {
        let xml = toast_xml(
            "a < b > c & d",
            "x & y < z > q",
            Some("https://x.test/?a=1&b=2\"'<>"),
        );
        assert!(xml.contains("a &lt; b"));
        assert!(xml.contains("&gt; c &amp; d"));
        assert!(xml.contains("x &amp; y"));
        assert!(xml.contains("&lt; z &gt; q"));
        assert!(xml.contains("&amp;b=2"));
        assert!(xml.contains("&quot;&apos;&lt;&gt;"));
    }

    #[test]
    fn omits_protocol_activation_without_click_url() {
        let xml = toast_xml("title", "body", None);
        assert!(!xml.contains("activationType=\"protocol\""));
        assert!(!xml.contains("launch="));
    }
}
