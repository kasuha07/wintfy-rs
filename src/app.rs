use std::{
    process::ExitCode,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use crate::{
    args::{Args, Command},
    config::{self, Config, LoadedConfig},
    ntfy::event::Notification,
    platform::{mutex::SingleInstance, paths, shell},
    tray::TrayHandle,
    worker::subscription::WorkerGroup,
};

#[derive(Debug, Clone)]
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
    },
    ConfigReloaded,
    PowerResume,
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

    let (tx, rx) = mpsc::channel();
    let tray = crate::tray::TrayApp::create(tx.clone())?;
    let tray_handle = tray.handle();
    if startup_config_error.is_some() {
        tray_handle.set_tooltip("wintfy-rs: config error");
    }
    let runtime_tx = tx.clone();
    let mut runtime = Runtime::new(loaded, tx, tray_handle);
    let event_thread = thread::Builder::new()
        .name("app-events".to_string())
        .spawn(move || runtime.run(rx))
        .map_err(|err| format!("failed to spawn event thread: {err}"))?;

    tray.message_loop();
    let _ = runtime_tx.send(AppEvent::TrayQuit);
    let _ = event_thread.join();
    Ok(ExitCode::SUCCESS)
}

struct Runtime {
    loaded: LoadedConfig,
    tx: Sender<AppEvent>,
    tray: TrayHandle,
    workers: Option<WorkerGroup>,
    muted: bool,
    shutdown_done: bool,
}

impl Runtime {
    fn new(loaded: LoadedConfig, tx: Sender<AppEvent>, tray: TrayHandle) -> Self {
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

    fn run(&mut self, rx: Receiver<AppEvent>) {
        while let Ok(event) = rx.recv() {
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
                    break;
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
                } => {
                    if self.muted {
                        log::debug!(
                            "notification from subscription {subscription} skipped because muted"
                        );
                        continue;
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
        }
        self.shutdown();
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
