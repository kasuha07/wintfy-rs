use std::{
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, INFINITE, ResetEvent, SetEvent},
        UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
            TranslateMessage, WM_QUIT,
        },
    },
    core::PCWSTR,
};

use crate::{
    args::{Args, Command},
    config::{self, Config, LoadedConfig},
    ntfy::event::Notification,
    platform::{mutex::SingleInstance, paths, shell},
    tray::TrayHandle,
    worker::subscription::WorkerGroup,
};

const MAX_QUEUED_NOTIFICATIONS: usize = 64;

#[derive(Debug)]
pub enum AppEvent {
    TrayReloadConfig,
    TrayOpenConfig,
    TrayOpenLogs,
    TrayTestNotification,
    TrayToggleMute,
    TrayToggleStartWithWindows,
    TrayQuit,
    SubscriptionConnected {
        name: String,
    },
    SubscriptionDisconnected {
        name: String,
        reason: String,
    },
    NtfyMessage {
        subscription: String,
        notification: Notification,
        permit: NotificationPermit,
    },
    ConfigReloaded,
    PowerResume,
}

#[derive(Clone)]
pub struct EventSender {
    tx: Sender<AppEvent>,
    wake: Arc<WakeEvent>,
    queued_notifications: Arc<AtomicUsize>,
}

impl EventSender {
    fn new(tx: Sender<AppEvent>, wake: Arc<WakeEvent>) -> Self {
        Self {
            tx,
            wake,
            queued_notifications: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn send(&self, event: AppEvent) -> bool {
        if self.tx.send(event).is_err() {
            return false;
        }
        self.wake.set();
        true
    }

    pub fn try_send_notification(&self, subscription: String, notification: Notification) -> bool {
        let Some(permit) = NotificationPermit::try_acquire(self.queued_notifications.clone())
        else {
            log::warn!("notification queue full, dropping notification from {subscription}");
            return false;
        };
        self.send(AppEvent::NtfyMessage {
            subscription,
            notification,
            permit,
        })
    }

    #[cfg(test)]
    fn new_for_test(tx: Sender<AppEvent>) -> Self {
        Self {
            tx,
            wake: Arc::new(WakeEvent {
                handle: HANDLE::default(),
            }),
            queued_notifications: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Debug)]
pub struct NotificationPermit {
    queued: Arc<AtomicUsize>,
}

impl NotificationPermit {
    fn try_acquire(queued: Arc<AtomicUsize>) -> Option<Self> {
        let mut current = queued.load(Ordering::Relaxed);
        loop {
            if current >= MAX_QUEUED_NOTIFICATIONS {
                return None;
            }
            match queued.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(Self { queued }),
                Err(value) => current = value,
            }
        }
    }
}

impl Drop for NotificationPermit {
    fn drop(&mut self) {
        self.queued.fetch_sub(1, Ordering::Relaxed);
    }
}

struct WakeEvent {
    handle: HANDLE,
}

unsafe impl Send for WakeEvent {}
unsafe impl Sync for WakeEvent {}

impl WakeEvent {
    fn create() -> Result<Arc<Self>, String> {
        let handle = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map_err(|err| format!("CreateEventW failed: {err}"))?;
        Ok(Arc::new(Self { handle }))
    }

    fn set(&self) {
        unsafe {
            let _ = SetEvent(self.handle);
        }
    }

    fn reset(&self) {
        unsafe {
            let _ = ResetEvent(self.handle);
        }
    }
}

impl Drop for WakeEvent {
    fn drop(&mut self) {
        if self.handle.is_invalid() {
            return;
        }
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn run() -> Result<ExitCode, String> {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(usage_or_err) => {
            eprintln!("{usage_or_err}");
            return Ok(ExitCode::FAILURE);
        }
    };

    match args.command {
        Command::Version => {
            println!("wintfy-rs {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Command::Install { startup } => {
            let _ = crate::logging::init_fallback();
            shell::install(startup)?;
            crate::toast::init_app_id()?;
            crate::toast::show_test()?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Uninstall { purge } => {
            let _ = crate::logging::init_fallback();
            shell::uninstall(purge)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::TestNotification => {
            let _ = crate::logging::init_fallback();
            crate::toast::init_app_id()?;
            crate::toast::show_test()?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Run => run_tray(args.config),
    }
}

fn run_tray(config_arg: Option<std::path::PathBuf>) -> Result<ExitCode, String> {
    let _instance = match SingleInstance::acquire()? {
        Some(instance) => instance,
        None => return Ok(ExitCode::SUCCESS),
    };

    let config_path = config_arg
        .clone()
        .unwrap_or(config::resolve_config_path(None)?);
    if !config_path.exists() {
        config::ensure_default_config(&config_path)?;
    }

    let mut startup_config_error = None;
    let loaded = match config::load_config(config_arg.as_deref()) {
        Ok(loaded) => loaded,
        Err(err) => {
            let _ = crate::logging::init_fallback();
            let redacted = crate::util::redact::redact_secret(&err);
            log::error!("configuration error: {redacted}");
            let _ = crate::toast::init_app_id();
            let _ = crate::toast::show_error("wintfy-rs config error", &redacted);
            startup_config_error = Some(redacted);
            LoadedConfig {
                path: config_path,
                config: Config {
                    subscriptions: Vec::new(),
                    ..Config::default()
                },
            }
        }
    };

    let _log_path = crate::logging::init(&loaded.config.app.log_level)
        .or_else(|_| crate::logging::init_fallback())?;
    log::info!("wintfy-rs starting");
    crate::toast::init_app_id()?;

    let (raw_tx, rx) = mpsc::channel();
    let wake = WakeEvent::create()?;
    let tx = EventSender::new(raw_tx, wake.clone());
    let tray = crate::tray::TrayApp::create(tx.clone())?;
    let tray_handle = tray.handle();
    if startup_config_error.is_some() {
        tray_handle.set_tooltip("wintfy-rs: config error");
    }
    let mut runtime = Runtime::new(loaded, tx, tray_handle);
    runtime.run(&tray, &rx, &wake);
    Ok(ExitCode::SUCCESS)
}

struct Runtime {
    loaded: LoadedConfig,
    tx: EventSender,
    tray: TrayHandle,
    workers: Option<WorkerGroup>,
    muted: bool,
    shutdown_done: bool,
}

impl Runtime {
    fn new(loaded: LoadedConfig, tx: EventSender, tray: TrayHandle) -> Self {
        let workers = if loaded.config.subscriptions.is_empty() {
            None
        } else {
            Some(WorkerGroup::start(&loaded.config, tx.clone()))
        };
        Self {
            loaded,
            tx,
            tray,
            workers,
            muted: false,
            shutdown_done: false,
        }
    }

    fn run(&mut self, tray: &crate::tray::TrayApp, rx: &Receiver<AppEvent>, wake: &WakeEvent) {
        let mut quit = false;
        while !quit {
            match wait_for_app_or_window_event(wake.handle) {
                Ok(LoopSignal::AppEvent) => {
                    wake.reset();
                    quit = self.drain_events(rx);
                }
                Ok(LoopSignal::WindowMessage) => {
                    if tray.dispatch_pending_messages() {
                        quit = true;
                    }
                    quit |= self.drain_events(rx);
                }
                Err(err) => {
                    log::error!("{err}");
                    break;
                }
            }
        }
        self.shutdown();
    }

    fn drain_events(&mut self, rx: &Receiver<AppEvent>) -> bool {
        let mut quit = false;
        while let Ok(event) = rx.try_recv() {
            if self.handle_event(event) {
                quit = true;
            }
        }
        quit
    }

    fn handle_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::TrayReloadConfig => self.reload_config(),
            AppEvent::TrayOpenConfig => {
                if let Err(err) = shell::open_path(&self.loaded.path) {
                    log::warn!("{err}");
                }
            }
            AppEvent::TrayOpenLogs => {
                if let Ok(path) = paths::log_file() {
                    let target = path.parent().unwrap_or(&path);
                    if let Err(err) = shell::open_path(target) {
                        log::warn!("{err}");
                    }
                }
            }
            AppEvent::TrayTestNotification => {
                if let Err(err) = crate::toast::show_test() {
                    log::warn!("test notification failed: {err}");
                }
            }
            AppEvent::TrayToggleMute => {
                self.muted = !self.muted;
                self.tray.set_muted(self.muted);
                self.tray.set_tooltip(if self.muted {
                    "wintfy-rs: muted"
                } else {
                    "wintfy-rs"
                });
                log::info!(
                    "notifications {}",
                    if self.muted { "muted" } else { "unmuted" }
                );
            }
            AppEvent::TrayToggleStartWithWindows => {
                match shell::set_startup_enabled(!shell::startup_enabled()) {
                    Ok(enabled) => {
                        self.tray.set_start_with_windows(enabled);
                        log::info!(
                            "start with Windows {}",
                            if enabled { "enabled" } else { "disabled" }
                        );
                    }
                    Err(err) => log::warn!("failed to change startup shortcut: {err}"),
                }
            }
            AppEvent::TrayQuit => {
                self.shutdown();
                self.tray.quit();
                return true;
            }
            AppEvent::SubscriptionConnected { name } => {
                log::info!("subscription {name} connected");
                self.tray.set_tooltip("wintfy-rs: connected");
            }
            AppEvent::SubscriptionDisconnected { name, reason } => {
                log::warn!("subscription {name} disconnected: {reason}");
                self.tray.set_tooltip("wintfy-rs: disconnected");
                if self.loaded.config.app.show_connection_status_toast {
                    let _ = crate::toast::show_error(
                        "wintfy-rs disconnected",
                        &format!("{name}: {reason}"),
                    );
                }
            }
            AppEvent::NtfyMessage {
                subscription,
                notification,
                permit: _permit,
            } => {
                if self.muted {
                    log::debug!(
                        "notification from subscription {subscription} skipped because muted"
                    );
                    return false;
                }
                log::debug!("notification from subscription {subscription}");
                if let Some(url) = notification.click_url.as_deref() {
                    log::debug!(
                        "notification has click URL {}",
                        crate::util::redact::redact_secret(url)
                    );
                }
                if let Err(err) = crate::toast::show(&notification) {
                    log::warn!("toast failed: {err}");
                }
            }
            AppEvent::ConfigReloaded => {}
            AppEvent::PowerResume => {
                log::info!("power resume detected, reconnecting subscriptions");
                self.restart_workers();
            }
        }
        false
    }

    fn reload_config(&mut self) {
        match config::load_config(Some(&self.loaded.path)) {
            Ok(loaded) => {
                self.tray.set_tooltip("wintfy-rs: reloading");
                self.stop_workers();
                self.loaded = loaded;
                self.workers = Some(WorkerGroup::start(&self.loaded.config, self.tx.clone()));
                log::info!("config reloaded");
                self.tray.set_tooltip("wintfy-rs: connecting");
                let _ = self.tx.send(AppEvent::ConfigReloaded);
            }
            Err(err) => {
                let redacted = crate::util::redact::redact_secret(&err);
                log::error!("config reload failed: {redacted}");
                let _ = crate::toast::show_error("wintfy-rs config error", &redacted);
            }
        }
    }

    fn restart_workers(&mut self) {
        self.stop_workers();
        if !self.loaded.config.subscriptions.is_empty() {
            self.workers = Some(WorkerGroup::start(&self.loaded.config, self.tx.clone()));
        }
    }

    fn stop_workers(&mut self) {
        if let Some(workers) = self.workers.take() {
            workers.stop(Duration::from_secs(3));
        }
    }

    fn shutdown(&mut self) {
        if self.shutdown_done {
            return;
        }
        self.shutdown_done = true;
        self.stop_workers();
        log::info!("wintfy-rs exiting");
        log::logger().flush();
    }
}

enum LoopSignal {
    AppEvent,
    WindowMessage,
}

fn wait_for_app_or_window_event(wake: HANDLE) -> Result<LoopSignal, String> {
    let wait = unsafe { MsgWaitForMultipleObjects(Some(&[wake]), false, INFINITE, QS_ALLINPUT) };
    if wait == WAIT_FAILED {
        return Err(format!("MsgWaitForMultipleObjects failed: {:?}", unsafe {
            GetLastError()
        }));
    }
    if wait == WAIT_OBJECT_0 {
        Ok(LoopSignal::AppEvent)
    } else {
        Ok(LoopSignal::WindowMessage)
    }
}

pub(crate) fn dispatch_pending_window_messages() -> bool {
    unsafe {
        let mut quit = false;
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                quit = true;
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(id: usize) -> Notification {
        Notification {
            id: id.to_string(),
            title: "title".to_string(),
            body: "body".to_string(),
            topic: "topic".to_string(),
            priority: 3,
            click_url: None,
        }
    }

    #[test]
    fn notification_queue_is_bounded_and_releases_on_drop() {
        let (tx, rx) = mpsc::channel();
        let sender = EventSender::new_for_test(tx);

        for id in 0..MAX_QUEUED_NOTIFICATIONS {
            assert!(sender.try_send_notification("sub".to_string(), notification(id)));
        }
        assert!(!sender.try_send_notification("sub".to_string(), notification(999)));

        drop(rx.try_recv().unwrap());
        assert!(sender.try_send_notification("sub".to_string(), notification(1000)));
    }
}
