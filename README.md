# wintfy-rs

[![CI](https://github.com/kasuha07/wintfy-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/kasuha07/wintfy-rs/actions/workflows/ci.yml)

`wintfy-rs` is a lightweight Windows tray client for [ntfy](https://ntfy.sh/). It reads a TOML config file, subscribes to one or more ntfy JSON streams, and shows native Windows Toast notifications. It is a single Rust executable with no .NET, Electron, WebView, Python, Node.js, JVM, or GUI framework dependency.

## Features

- Windows 10/11 x64 tray app using `Shell_NotifyIcon`.
- Native Windows Toast notifications with AppUserModelID support.
- ntfy HTTP JSON stream subscriptions.
- Multiple subscriptions, with topics on the same subscription merged into one stream URL.
- Public or private ntfy servers.
- `none`, `bearer`, and `basic` authentication.
- Automatic reconnect with exponential backoff and jitter.
- Config reload from the tray menu.
- Tray menu actions for Open config, Reload config, Logs, Test notification, Mute/Unmute, Start with Windows, and Quit.
- Single-instance mutex.
- Rotating file logs with secret redaction.
- User-local install and uninstall commands.

## Download

For local builds:

```powershell
cargo build --release
.\scripts\package.ps1
```

The zip is written to `dist\wintfy-rs-v0.1.0-windows-x64.zip`.

## Install

Run from PowerShell:

```powershell
.\wintfy-rs.exe --install
```

Install is per-user and does not require administrator rights. It copies the exe to:

```text
%LOCALAPPDATA%\Programs\wintfy-rs\wintfy-rs.exe
```

It also creates:

```text
%APPDATA%\wintfy-rs\config.toml
%APPDATA%\Microsoft\Windows\Start Menu\Programs\wintfy-rs.lnk
%LOCALAPPDATA%\wintfy-rs\logs\
```

To install and also start with Windows:

```powershell
.\wintfy-rs.exe --install --startup
```

## Usage

Start the tray client:

```powershell
wintfy-rs.exe
```

Use a custom config:

```powershell
wintfy-rs.exe --config C:\Users\me\AppData\Roaming\wintfy-rs\config.toml
```

Other commands:

```powershell
wintfy-rs.exe --test-notification
wintfy-rs.exe --version
wintfy-rs.exe --uninstall
wintfy-rs.exe --uninstall --purge
```

## Configuration

Config lookup order:

1. `--config PATH`
2. `config.toml` next to the exe
3. `%APPDATA%\wintfy-rs\config.toml`

Minimal config:

```toml
[app]
log_level = "info"

[[subscriptions]]
name = "default"
server = "https://ntfy.sh"
topics = ["alerts", "backup"]
```

Private server with bearer token:

```toml
[[subscriptions]]
name = "private-server"
server = "https://ntfy.example.com"
topics = ["ops", "deploy"]
auth = "bearer"
token = "tk_xxxxxxxxxxxxxxxxx"
```

Private server with basic auth:

```toml
[[subscriptions]]
name = "basic-auth-server"
server = "https://ntfy.example.net"
topics = ["home"]
auth = "basic"
username = "user"
password = "pass"
```

## Subscribe to ntfy.sh

Pick a topic name and add it to the config:

```toml
[[subscriptions]]
name = "default"
server = "https://ntfy.sh"
topics = ["my-topic"]
```

Publish a test message:

```powershell
Invoke-WebRequest -Method Post -Body "hello from wintfy-rs" https://ntfy.sh/my-topic
```

Then reload config from the tray menu or restart `wintfy-rs.exe`.

## Logs

Logs are written to:

```text
%LOCALAPPDATA%\wintfy-rs\logs\wintfy-rs.log
```

The tray menu has a `Logs` item that opens the log folder. Logs rotate at 1 MB and keep three backups.

Tokens, passwords, Authorization headers, and URL query strings are redacted before logging.

## Start With Windows

Use:

```powershell
wintfy-rs.exe --install --startup
```

This creates:

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\wintfy-rs.lnk
```

## Uninstall

Remove shortcuts:

```powershell
wintfy-rs.exe --uninstall
```

Remove shortcuts and the installed exe:

```powershell
wintfy-rs.exe --uninstall --purge
```

Config and logs are preserved unless you delete them manually.

## FAQ

**Toast notifications do not appear.**  
Run `wintfy-rs.exe --install`, then `wintfy-rs.exe --test-notification`. Desktop Toast reliability depends on the Start Menu shortcut and matching AppUserModelID.

**Can I use a server path prefix?**  
Yes. A server such as `https://example.com/ntfy` becomes `https://example.com/ntfy/topic/json`.

**Are arbitrary commands supported from notifications?**  
No. Click URLs are limited to `http` and `https` by default. The app does not execute shell commands from ntfy messages.

**Does it download attachments?**  
No. Attachment name and size are appended to the notification body.

## License

MIT.
