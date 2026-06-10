# ntfy Windows Lightweight Tray Client 开发 SPEC

## 1. 项目概述

本项目是一个面向 Windows 10 的极轻量 ntfy 客户端。程序不提供完整 GUI，仅通过配置文件工作，常驻系统托盘，订阅一个或多个 ntfy topic，并在收到消息时使用 Windows 原生 Toast 通知展示。

项目核心目标是：

1. 单 exe 分发。
2. 不依赖 .NET Runtime、Electron、WebView、Python、Node.js 或 JVM。
3. 支持开机自启动设置。
4. 常驻内存占用尽可能低。
5. 程序体积尽可能小。
6. 支持 Windows 10/11 原生通知，优先支持本机环境 Windows 10。
7. 支持托盘图标、托盘菜单、配置热重载和退出。
8. 支持 ntfy 官方 HTTP JSON stream 订阅协议。
9. 支持私有 ntfy server、token/basic auth、多个 topic。
10. 在网络中断、服务器重启、休眠唤醒后自动恢复连接。

项目暂定名称：

```text
wintfy-rs
```

## 2. 目标平台

### 2.1 支持平台

最低支持：

```text
Windows 10 1809 x64
```

优先支持：

```text
Windows 10 x64
Windows 11 x64
```

### 2.2 不支持平台

```text
Windows 7
Windows 8 / 8.1
Linux
macOS
ARM64 Windows
Windows ARM64
```

## 3. 推荐技术栈

### 3.1 主技术栈

```text
Language: Rust
Target: x86_64-pc-windows-msvc
Windows API: windows-sys 或 windows crate
Networking: blocking HTTP JSON stream
Tray icon: Win32 Shell_NotifyIcon
Toast: WinRT Windows.UI.Notifications
Config: TOML
Serialization: serde + serde_json
Logging: tracing 或自定义极简 logger
Build: cargo + release profile optimization
```

### 3.2 明确不使用

```text
.NET / WPF / WinForms
Electron
Tauri
WebView2
Qt
GTK
Tokio
WebSocket 作为默认订阅方式
YAML 配置
SQLite
本地 HTTP server
完整 GUI 框架
```

### 3.3 选型理由

Rust 适合作为默认实现语言，原因如下：

1. 无 GC，常驻内存更可控。
2. 可生成单 exe。
3. 可直接调用 Win32 和 WinRT API。
4. 相比 C/C++，内存安全性更好。
5. 相比 Go，运行时和基础内存占用更容易压低。
6. 相比 .NET/Electron，分发体积和运行依赖更小。

## 4. 产品形态

### 4.1 用户体验

程序启动后：

1. 读取配置文件。
2. 不创建窗口。
3. 注册托盘图标。
4. 注册或检查 Toast 所需 AppUserModelID。
5. 建立 ntfy JSON stream 订阅连接。
6. 收到 ntfy message 后弹出 Windows Toast。
7. 用户可通过托盘菜单执行：
   - Open config
   - Mute / Unmute
   - Logs
   - [x] Start with Windows
   - Quit

### 4.2 运行模式

程序支持三种模式：

```text
normal mode      常驻托盘运行
install mode     创建开始菜单快捷方式、设置 AUMID、可选设置开机启动
uninstall mode   移除快捷方式、开机启动项和相关注册信息
```

命令行示例：

```powershell
wintfy-rs.exe
wintfy-rs.exe --config C:\Users\me\AppData\Roaming\wintfy-rs\config.toml
wintfy-rs.exe --install
wintfy-rs.exe --uninstall
wintfy-rs.exe --test-notification
wintfy-rs.exe --version
```

## 5. 功能需求

## 5.1 配置文件

### 5.1.1 配置文件位置

默认配置路径：

```text
%APPDATA%\wintfy-rs\config.toml
```

便携模式配置路径：

```text
.\config.toml
```

配置查找顺序：

1. 命令行 `--config` 指定的路径。
2. exe 同目录下的 `config.toml`。
3. `%APPDATA%\wintfy-rs\config.toml`。

### 5.1.2 配置文件示例

```toml
[app]
log_level = "info"

[[subscriptions]]
name = "default"
server = "https://ntfy.sh"
topics = ["alerts", "backup"]

[[subscriptions]]
name = "private-server"
server = "https://ntfy.example.com"
topics = ["ops", "deploy"]
auth = "bearer"
token = "tk_xxxxxxxxxxxxxxxxx"

[[subscriptions]]
name = "basic-auth-server"
server = "https://ntfy.example.net"
topics = ["home"]
auth = "basic"
username = "user"
password = "pass"
```

### 5.1.3 配置字段说明

#### subscriptions

| 字段 | 类型 | 必填 | 说明 |
|---|---:|---:|---|
| name | string | 是 | 订阅名称 |
| server | string | 是 | ntfy server 根地址 |
| topics | array | 是 | topic 列表 |
| auth | string | 否 | `none/bearer/basic` |
| token | string | 否 | Bearer token |
| username | string | 否 | Basic auth 用户名 |
| password | string | 否 | Basic auth 密码 |

## 5.2 ntfy 订阅

### 5.2.1 默认协议

MVP 使用 HTTP JSON stream。

请求格式：

```text
GET {server}/{topic1},{topic2}/json
```

示例：

```text
GET https://ntfy.sh/alerts,backup/json
```

### 5.2.2 消息读取

程序应以流式方式逐行读取响应体。

每一行应尝试解析为 JSON。

支持的 ntfy event 类型：

```text
open
keepalive
message
poll_request
```

MVP 行为：

| event | 行为 |
|---|---|
| open | 标记连接成功 |
| keepalive | 默认忽略 |
| message | 显示 Toast |
| poll_request | 默认忽略 |

### 5.2.3 消息字段

MVP 至少解析以下字段：

```json
{
  "id": "string",
  "time": 1234567890,
  "event": "message",
  "topic": "alerts",
  "title": "Title",
  "message": "Message body",
  "priority": 3,
  "tags": ["warning"],
  "click": "https://example.com",
  "attachment": {
    "name": "file.txt",
    "type": "text/plain",
    "size": 1234,
    "url": "https://example.com/file.txt"
  }
}
```

字段处理规则：

1. `title` 为空时，使用 topic 作为标题。
2. `message` 为空时，不弹出空通知，除非存在 attachment 或 click。
3. `priority` 小于 `notification.min_priority` 时忽略。
4. `tags` 可转为 emoji 前缀，但必须可关闭。
5. `click` 存在时，Toast 点击打开 URL。
6. `attachment` 存在时，在正文追加附件名称和大小。
7. 未知字段忽略，不应导致崩溃。

### 5.2.4 多订阅策略

每个 `[[subscriptions]]` 建立一个 worker thread。

同一 server 下多个 topic 应合并为一个请求：

```text
/server/topic1,topic2,topic3/json
```

这样可以减少连接数量和内存占用。

### 5.2.5 重连策略

连接失败或流中断时：

1. 记录错误。
2. 更新托盘状态为 disconnected。
3. 按指数退避重连。
4. 最大退避不超过 `reconnect_max_seconds`。
5. 成功连接后重置退避。
6. 电脑从休眠恢复后应尽快重连。

退避算法：

```text
delay = min(initial * 2^attempt, max)
if jitter:
    delay = delay * random(0.8, 1.2)
```

### 5.2.6 认证

支持三种认证：

```text
none
bearer
basic
```

Bearer auth：

```http
Authorization: Bearer <token>
```

Basic auth：

```http
Authorization: Basic base64(username:password)
```

认证信息不得写入日志。

## 5.3 Windows 托盘

### 5.3.1 托盘实现

使用 Win32 `Shell_NotifyIcon`。

程序应创建一个隐藏窗口，用于接收托盘消息。

隐藏窗口不显示在任务栏。

托盘图标状态：

| 状态 | 表现 |
|---|---|
| connecting | 默认图标，可选灰色覆盖 |
| connected | 默认图标 |
| disconnected | 默认图标，可选红点覆盖 |
| error | 默认图标，可选警告覆盖 |

MVP 可只使用一个内嵌 ico 资源，后续再增加状态图标。

### 5.3.4 Explorer 重启处理

Windows Explorer 重启后，托盘图标会消失。

程序必须处理 `TaskbarCreated` 注册消息，并在收到该消息后重新添加托盘图标。

## 5.4 Windows Toast 通知

### 5.4.1 Toast 方案

使用 Windows 原生 Toast。

要求：

1. 设置 AppUserModelID。
2. 创建开始菜单快捷方式。
3. 快捷方式中写入相同 AppUserModelID。
4. Toast 激活时可处理点击事件。

### 5.4.2 安装命令

`--install` 应执行：

1. 创建配置目录。
2. 创建默认配置文件，如果不存在。
3. 创建开始菜单快捷方式：
   ```text
   %APPDATA%\Microsoft\Windows\Start Menu\Programs\wintfy-rs.lnk
   ```
4. 将 AppUserModelID 写入快捷方式属性。
5. 可选创建开机启动快捷方式：
   ```text
   %APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\wintfy-rs.lnk
   ```
6. 弹出测试通知验证 Toast 是否工作。

### 5.4.3 Toast 内容格式

标题：

```text
{title}
```

正文：

```text
{message}
```

附加信息：

```text
Topic: {topic}
Priority: {priority}
Attachment: {attachment.name}
```

Toast XML 结构应尽量简单，MVP 不使用图片。

### 5.4.4 Toast 点击行为

点击 Toast 时：

1. 如果 ntfy message 存在 `click` 字段，打开对应 URL。
2. 否则不执行操作。
3. 如果 URL scheme 非 `http` 或 `https`，默认拒绝，除非配置中显式允许。

允许的 URL scheme：

```text
http
https
```

后续可选：

```text
mailto
file
custom scheme
```

### 5.4.5 通知降噪

必须支持以下过滤：

1. 按 priority 过滤。
2. 忽略 keepalive。
3. 限制标题长度。
4. 限制正文长度。
5. 相同 message id 不重复提醒。

重复消息缓存：

```text
LRU cache, max 256 message ids
```

## 5.5 日志

### 5.5.1 日志路径

默认日志路径：

```text
%LOCALAPPDATA%\wintfy-rs\logs\wintfy-rs.log
```

### 5.5.2 日志轮转

MVP 日志轮转规则：

```text
单文件最大 1 MB
最多保留 3 个旧日志
```

### 5.5.3 日志级别

支持：

```text
error
warn
info
debug
trace
```

默认：

```text
info
```

### 5.5.4 敏感信息处理

日志中不得出现：

```text
token
password
Authorization header
完整 Basic auth 内容
```

URL 中如果包含敏感 query，应脱敏。

## 5.6 单实例

只允许一个实例运行。

实现方式：

```text
CreateMutexW("Global\\wintfy-rs")
```

当第二个实例启动时：

1. 检测已有实例。
2. 向已有实例发送激活消息。
3. 退出当前实例。

MVP 可简化为：

```text
检测到已有实例后直接退出
```

## 5.7 配置热重载

Reload 行为：

1. 读取新配置。
2. 校验配置。
3. 如校验失败，保留旧配置和旧连接。
4. 如校验成功，停止旧 worker。
5. 启动新 worker。
6. 更新托盘状态。
7. 写日志。

## 5.8 优雅退出

用户点击 Quit 后：

1. 设置全局 shutdown flag。
2. 移除托盘图标。
3. 通知所有 worker 停止。
4. 关闭 HTTP stream。
5. flush 日志。
6. 退出进程。

退出超时：

```text
最多等待 3 秒
```

超时后强制退出。

## 6. 非功能需求

## 6.1 程序体积目标

Release x64 单 exe 目标：

```text
理想目标: <= 2 MB
可接受目标: <= 5 MB
硬上限: <= 10 MB
```

注意：

使用 `windows` crate、serde、toml、HTTP client 后，体积可能超过 2 MB。

如需极限体积，应逐步替换为：

```text
windows-sys 替代 windows
WinHTTP 替代高级 HTTP client
INI/JSON 替代 TOML
自定义 logger 替代 tracing
```

## 6.2 RAM 占用目标

空闲常驻内存目标：

```text
理想目标: <= 5 MB private working set
可接受目标: <= 15 MB private working set
硬上限: <= 30 MB private working set
```

连接数量：

```text
每个 subscription 一个 HTTP stream
默认建议 <= 3 个 subscription
```

## 6.3 CPU 占用目标

空闲状态：

```text
接近 0%
```

收到消息时：

```text
短暂上升，完成 Toast 后回到 0%
```

不得使用忙轮询。

## 6.4 网络占用

长连接空闲时只接收 keepalive。

不得主动高频轮询。

断线重连必须指数退避，避免服务器不可用时频繁请求。

## 6.5 稳定性

程序应满足：

1. 网络断开不崩溃。
2. ntfy server 返回非 200 不崩溃。
3. JSON 解析失败不崩溃。
4. Toast 失败不影响订阅。
5. Explorer 重启后托盘图标恢复。
6. 电脑休眠/唤醒后连接恢复。
7. 配置错误时保留旧配置继续运行。

## 7. 安全要求

## 7.1 配置文件权限

创建配置文件时，应尽量限制为当前用户可读写。

MVP 至少要求：

```text
配置文件位于用户 AppData 目录
不写入 Program Files
不要求管理员权限
```

## 7.2 凭据处理

1. token/password 只从配置文件读取。
2. 不输出到日志。
3. 不在 Toast 中显示。
4. 不在 panic/error message 中显示。
5. 不上传任何 telemetry。

后续可选：

```text
Windows Credential Manager
DPAPI 加密 token
```

MVP 不强制实现凭据加密。

## 7.3 URL 打开安全

Toast 点击打开 URL 时：

1. 默认仅允许 `http` 和 `https`。
2. URL 长度应有限制。
3. 非法 URL 不打开，只记录 warning。
4. 不执行 shell command。
5. 不支持任意命令 action。

## 7.4 自动更新

MVP 不实现自动更新。

理由：

1. 降低安全风险。
2. 降低体积和复杂度。
3. 避免额外后台网络请求。

## 8. 架构设计

## 8.1 进程结构

```text
main thread
  ├─ init logging
  ├─ parse args
  ├─ load config
  ├─ create single instance mutex
  ├─ create hidden window
  ├─ register tray icon
  ├─ init toast
  ├─ spawn subscription workers
  └─ Win32 message loop

worker thread per subscription
  ├─ build HTTP request
  ├─ open JSON stream
  ├─ read line by line
  ├─ parse ntfy event
  ├─ filter message
  ├─ send notification event to main thread
  └─ reconnect on failure
```

## 8.2 模块划分

建议目录结构：

```text
src/
  main.rs
  args.rs
  config.rs
  app.rs
  logging.rs
  tray/
    mod.rs
    win32.rs
    menu.rs
    icon.rs
  toast/
    mod.rs
    winrt.rs
    shortcut.rs
    xml.rs
  ntfy/
    mod.rs
    client.rs
    event.rs
    auth.rs
  worker/
    mod.rs
    subscription.rs
    reconnect.rs
  platform/
    mod.rs
    paths.rs
    mutex.rs
    shell.rs
    power.rs
  util/
    mod.rs
    redact.rs
    lru.rs
```

## 8.3 内部事件模型

使用标准库 channel。

```rust
enum AppEvent {
    TrayReloadConfig,
    TrayOpenConfig,
    TrayOpenLogs,
    TrayTestNotification,
    TrayQuit,
    SubscriptionConnected { name: String },
    SubscriptionDisconnected { name: String, reason: String },
    NtfyMessage { subscription: String, message: NtfyMessage },
    ConfigReloaded,
    FatalError(String),
}
```

主线程负责：

1. 处理托盘菜单事件。
2. 处理 worker 发来的消息。
3. 调用 Toast。
4. 更新托盘状态。
5. 执行退出。

worker 线程不得直接操作 UI 或托盘。

## 8.4 同步与关闭

使用：

```text
Arc<AtomicBool> shutdown_flag
std::sync::mpsc channel
JoinHandle
```

关闭流程：

```text
shutdown_flag = true
drop/close HTTP connection
join worker threads with timeout
remove tray icon
exit message loop
```

## 9. ntfy 消息模型

Rust 结构示意：

```rust
#[derive(Debug, Deserialize)]
struct NtfyEvent {
    id: Option<String>,
    time: Option<i64>,
    event: String,
    topic: Option<String>,
    title: Option<String>,
    message: Option<String>,
    priority: Option<i32>,
    tags: Option<Vec<String>>,
    click: Option<String>,
    attachment: Option<NtfyAttachment>,
}

#[derive(Debug, Deserialize)]
struct NtfyAttachment {
    name: Option<String>,
    r#type: Option<String>,
    size: Option<u64>,
    url: Option<String>,
}
```

内部通知模型：

```rust
struct Notification {
    id: String,
    title: String,
    body: String,
    topic: String,
    priority: i32,
    click_url: Option<String>,
}
```

## 10. HTTP 实现要求

## 10.1 MVP 实现

优先使用阻塞式 HTTP。

推荐实现路径：

```text
方案 A: WinHTTP + windows-sys
方案 B: ureq
```

MVP 可以先用方案 B 降低开发成本。后续为降低体积和依赖，可切换到方案 A。

## 10.2 请求头

默认请求头：

```http
User-Agent: wintfy-rs/{version}
Accept: application/x-ndjson, application/json, */*
```

认证请求头按配置添加。

## 10.3 HTTP 状态处理

| 状态码 | 行为 |
|---|---|
| 200 | 开始读取 stream |
| 401/403 | 记录认证错误，按较慢退避重连 |
| 404 | 记录 topic/server 错误，按较慢退避重连 |
| 429 | 按较慢退避重连 |
| 5xx | 指数退避重连 |
| 其他 | 记录并重连 |

## 10.4 行读取限制

单行 JSON 最大长度：

```text
默认 1 MB
```

超过限制：

1. 丢弃该行。
2. 记录 warning。
3. 连接可继续。

## 11. 构建配置

## 11.1 Cargo release profile

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

## 11.2 Windows subsystem

正式构建应使用 Windows subsystem，避免启动时出现控制台窗口。

Cargo 配置或源码中设置：

```rust
#![windows_subsystem = "windows"]
```

Debug 构建可保留 console，便于排查。

## 11.3 Feature flags

建议提供 feature flags：

```toml
[features]
default = ["toml-config", "native-toast"]
toml-config = []
native-toast = []
winhttp = []
ureq-client = []
debug-console = []
```

目标：

1. release 默认无 console。
2. 可切换 HTTP backend。
3. 可在 debug 模式下输出 console 日志。
4. 可裁剪不需要的功能。

## 12. 安装与分发

## 12.1 MVP 分发形式

MVP 提供 zip：

```text
wintfy-rs-v0.1.0-windows-x64.zip
  wintfy-rs.exe
  config.example.toml
  README.md
  LICENSE
```

用户执行：

```powershell
wintfy-rs.exe --install
```

## 12.2 安装行为

`--install` 不需要管理员权限。

安装到用户目录：

```text
%LOCALAPPDATA%\Programs\wintfy-rs\wintfy-rs.exe
%APPDATA%\wintfy-rs\config.toml
%LOCALAPPDATA%\wintfy-rs\logs\
```

创建快捷方式：

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\wintfy-rs.lnk
```

可选开机启动：

```text
%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\wintfy-rs.lnk
```

## 12.3 卸载行为

`--uninstall` 应删除：

1. 开始菜单快捷方式。
2. 开机启动快捷方式。
3. 可选删除安装目录中的 exe。

默认不删除：

```text
config.toml
logs
```

除非用户传入：

```powershell
wintfy-rs.exe --uninstall --purge
```

## 13. 错误处理

## 13.1 配置错误

配置错误分为：

```text
fatal config error
recoverable config error
```

启动时配置错误：

1. 写日志。
2. 弹出错误 Toast，如果 Toast 可用。
3. 托盘图标进入 error 状态。
4. 程序继续运行，允许用户通过托盘打开配置。

热重载配置错误：

1. 保留旧配置。
2. 保留旧连接。
3. 弹出错误通知。
4. 写日志。

## 13.2 Toast 错误

Toast 失败时：

1. 写日志。
2. 不影响订阅连接。
3. 托盘状态不必变 error，除非连续失败。

## 13.3 网络错误

网络错误时：

1. 写日志。
2. 更新 subscription 状态。
3. 按重连策略处理。
4. 默认不弹 Toast，避免断网时刷屏。

只有在 `show_connection_status_toast = true` 时才弹连接状态通知。

## 14. 性能优化要求

## 14.1 依赖控制

依赖原则：

1. 每增加一个 crate，都必须有明确理由。
2. 不引入 async runtime，除非证明收益大于成本。
3. 不引入 GUI 框架。
4. 不引入大型日志框架，除非必要。
5. 不引入 YAML parser。
6. 不引入数据库。

## 14.2 内存优化

1. 逐行读取 HTTP stream，不缓存完整响应。
2. 消息处理后立即释放。
3. 重复消息缓存限制大小。
4. 不保存完整历史消息。
5. 不下载附件。
6. 不缓存图片。
7. 不创建多余线程。

## 14.3 体积优化

1. release 使用 LTO。
2. panic abort。
3. strip symbols。
4. 尽量使用 `windows-sys` 精准绑定。
5. 避免默认启用大型 crate feature。
6. 避免 TLS 重复实现；优先使用系统 TLS 或 WinHTTP。

## 15. 测试计划

## 15.1 单元测试

覆盖：

```text
config parsing
config validation
URL building
auth header generation
JSON event parsing
priority filtering
message truncation
duplicate message filtering
reconnect backoff
secret redaction
```

## 15.2 集成测试

覆盖：

1. mock ntfy JSON stream。
2. stream 正常收到 message。
3. stream 中断后自动重连。
4. server 返回 401/403。
5. server 返回 5xx。
6. 非法 JSON 行。
7. 超长 JSON 行。
8. 多 topic 合并 URL。
9. 配置热重载。

## 15.3 Windows 手工测试

...

## 15.4 性能测试

测试指标：

```text
exe size
private working set
handle count
thread count
idle CPU
network reconnect behavior
```

目标：

```text
exe size <= 5 MB
private working set <= 20 MB
idle CPU ~= 0%
threads <= 5 + subscription_count
```

测试工具：

```text
Task Manager
Process Explorer
Windows Performance Recorder
```

## 16. 版本规划

## 16.1 MVP v0.1

必须完成：

1. 配置文件读取。
2. 单实例。
3. 托盘图标。
4. 托盘菜单 Quit / Reload config / Open config。
5. ntfy JSON stream 订阅。
6. Bearer token。
7. Basic auth。
8. Toast 通知。
9. 自动重连。
10. 日志。
11. `--install` 创建快捷方式和 AUMID。
12. zip 分发。

## 16.2 v0.2

增加：

1. 开机启动菜单。
2. 连接状态图标。
3. Test notification。
4. Explorer 重启恢复托盘。
5. 日志轮转。
6. 休眠唤醒处理。
7. 配置校验错误 Toast。
8. 重复消息过滤。

## 16.3 v0.3

增加：

1. Windows Credential Manager 支持。
2. DPAPI token 加密。
3. 附件信息展示优化。
4. action button。
5. 多语言。
6. Windows ARM64 构建。

## 16.4 暂不计划

```text
完整 GUI 设置界面
消息历史数据库
附件下载
消息回复
自动更新
跨平台支持
Electron/Tauri 前端
```

## 17. 验收标准

MVP 完成标准：

1. 在干净 Windows 10 x64 上可运行。
2. 不需要安装 .NET Runtime。
3. 不需要管理员权限。
4. 启动后显示托盘图标。
5. 可通过配置订阅 ntfy.sh topic。
6. 收到消息后弹出 Windows Toast。
7. 私有 server + bearer token 可用。
8. 网络断开后不会崩溃，并能自动重连。
9. Quit 后进程完全退出。
10. release exe 小于 10 MB。
11. 空闲 private working set 小于 30 MB。
12. token/password 不出现在日志中。

## 18. 风险与注意事项

## 18.1 Toast 可靠性

Windows desktop Toast 对 AppUserModelID 和快捷方式有要求。未执行 `--install` 时，Toast 可能无法稳定显示。

应提供：

```text
wintfy-rs.exe --install
wintfy-rs.exe --test-notification
```

用于修复和验证 Toast 环境。

## 18.2 Windows API 复杂度

直接调用 Win32/WinRT 会增加实现复杂度。

建议开发顺序：

1. 先实现配置和 ntfy stream。
2. 再实现托盘。
3. 再实现 Toast。
4. 最后做 install/uninstall。

## 18.3 体积与开发效率冲突

为了快速完成 MVP，可以接受：

```text
ureq
toml
serde
```

为了后续极致优化，可替换为：

```text
WinHTTP
INI 或极简 JSON
windows-sys only
custom minimal logger
```

## 18.4 ntfy 兼容性

应优先兼容官方 ntfy server。私有 server 可能有反向代理、TLS、认证、路径前缀差异。

配置中 `server` 必须允许：

```text
https://ntfy.example.com
https://example.com/ntfy
```

URL 拼接必须正确处理尾部 slash 和 path prefix。

## 19. 推荐开发顺序

### Phase 1: Core

1. 建立 Rust 项目。
2. 定义 config schema。
3. 实现配置读取和校验。
4. 实现 ntfy JSON stream client。
5. 实现 mock stream 测试。
6. 实现 message filter。

### Phase 2: Windows shell

1. 创建隐藏窗口。
2. 实现 Win32 message loop。
3. 实现 Shell_NotifyIcon。
4. 实现托盘菜单。
5. 实现 Quit。
6. 实现 Explorer 重启后恢复托盘。

### Phase 3: Toast

1. 实现 AppUserModelID。
2. 实现开始菜单快捷方式创建。
3. 实现 Toast XML。
4. 实现测试通知。
5. 实现 message 到 Toast 的转换。
6. 实现点击打开 URL。

### Phase 4: Runtime quality

1. 实现自动重连。
2. 实现配置热重载。
3. 实现日志轮转。
4. 实现单实例。
5. 实现休眠唤醒处理。
6. 实现敏感信息脱敏。

### Phase 5: Release

1. 设置 release profile。
2. 设置 Windows subsystem。
3. 添加 ico 资源。
4. 构建 zip。
5. 在 Windows 10 测试。
6. 编写 README。
7. 发布 v0.1.0。

## 20. README 最小内容

README 至少包含：

```text
项目介绍
功能列表
下载方式
安装方式
配置示例
如何订阅 ntfy.sh
如何使用私有 server
如何设置 token
如何开机启动
如何查看日志
常见问题
卸载方式
```

## 21. License

建议使用：

```text
MIT
```

或：

```text
Apache-2.0
```

如使用第三方 crate，需要检查依赖 license，确保可分发。

## 22. 最终技术决策

默认实现路线：

```text
Rust
+ x86_64-pc-windows-msvc
+ windows-sys/windows
+ blocking HTTP JSON stream
+ serde_json
+ TOML config
+ Shell_NotifyIcon tray
+ native Windows Toast
+ no GUI framework
+ no async runtime
+ no .NET
+ no Electron
```

项目成功标准：

```text
像命令行工具一样简单
像托盘工具一样安静
像原生 Windows app 一样弹通知
像系统服务一样稳定
但不引入服务安装、GUI 框架或大型运行时
```
