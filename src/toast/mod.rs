use windows::{
    Data::Xml::Dom::XmlDocument,
    UI::Notifications::{ToastNotification, ToastNotificationManager},
    Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID,
    core::{HSTRING, PCWSTR},
};

use crate::{ntfy::event::Notification, platform::paths};

pub fn init_app_id() -> Result<(), String> {
    let id = wide(paths::AUMID);
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(PCWSTR(id.as_ptr()))
            .map_err(|err| format!("SetCurrentProcessExplicitAppUserModelID failed: {err}"))?;
    }
    Ok(())
}

pub fn show(notification: &Notification) -> Result<(), String> {
    let xml = toast_xml(
        &notification.title,
        &notification.body,
        notification.click_url.as_deref(),
    );
    show_xml(&xml)
}

pub fn show_test() -> Result<(), String> {
    show_xml(&toast_xml("wintfy-rs", "Test notification", None))
}

pub fn show_error(title: &str, body: &str) -> Result<(), String> {
    show_xml(&toast_xml(title, body, None))
}

fn show_xml(xml: &str) -> Result<(), String> {
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

fn toast_xml(title: &str, body: &str, launch: Option<&str>) -> String {
    let activation_attrs = launch
        .map(|url| {
            format!(
                r#" activationType="protocol" launch="{}""#,
                xml_escape_attr(url)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<toast{activation_attrs}><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        xml_escape_text(title),
        xml_escape_text(body)
    )
}

fn xml_escape_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_escape_attr(input: &str) -> String {
    xml_escape_text(input)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml() {
        let xml = toast_xml("a < b", "x & y", Some("https://x.test/?a=1&b=2"));
        assert!(xml.contains("a &lt; b"));
        assert!(xml.contains("x &amp; y"));
        assert!(xml.contains("&amp;b=2"));
    }

    #[test]
    fn omits_protocol_activation_without_click_url() {
        let xml = toast_xml("title", "body", None);
        assert!(!xml.contains("activationType=\"protocol\""));
        assert!(!xml.contains("launch="));
    }
}
