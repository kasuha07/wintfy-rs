use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    app::AppEvent,
    config::{Config, SubscriptionConfig},
    ntfy::{
        client::{self, StreamItem},
        filter::MessageFilter,
    },
    worker::reconnect::Backoff,
};

pub struct WorkerGroup {
    shutdown: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl WorkerGroup {
    pub fn start(config: &Config, tx: Sender<AppEvent>) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        for sub in config.subscriptions.clone() {
            let tx = tx.clone();
            let worker_shutdown = shutdown.clone();
            let notification = config.notification.clone();
            let security = config.security.clone();
            let network = config.network.clone();
            let handle = thread::Builder::new()
                .name(format!("ntfy-{}", sub.name))
                .spawn(move || {
                    run_worker(
                        sub,
                        notification,
                        security,
                        network.reconnect_initial_seconds,
                        network.reconnect_max_seconds,
                        network.reconnect_jitter,
                        network.line_max_bytes,
                        tx,
                        worker_shutdown,
                    );
                });
            match handle {
                Ok(handle) => handles.push(handle),
                Err(err) => log::error!("failed to spawn subscription worker: {err}"),
            }
        }
        Self { shutdown, handles }
    }

    pub fn stop(self, timeout: Duration) {
        self.shutdown.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + timeout;
        for handle in self.handles {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            join_with_timeout(handle, remaining);
        }
    }
}

fn run_worker(
    sub: SubscriptionConfig,
    notification: crate::config::NotificationConfig,
    security: crate::config::SecurityConfig,
    reconnect_initial_seconds: u64,
    reconnect_max_seconds: u64,
    reconnect_jitter: bool,
    line_max_bytes: usize,
    tx: Sender<AppEvent>,
    shutdown: Arc<AtomicBool>,
) {
    let agent = match client::agent(security.skip_tls_verify) {
        Ok(agent) => agent,
        Err(err) => {
            log::error!(
                "subscription {} cannot initialize HTTP client: {err}",
                sub.name
            );
            let _ = tx.send(AppEvent::SubscriptionDisconnected {
                name: sub.name,
                reason: err,
            });
            return;
        }
    };
    let mut backoff = Backoff::new(
        reconnect_initial_seconds,
        reconnect_max_seconds,
        reconnect_jitter,
    );
    let mut filter = MessageFilter::new(notification, security);
    let version = env!("CARGO_PKG_VERSION");

    while !shutdown.load(Ordering::Relaxed) {
        let name = sub.name.clone();
        let result = client::read_stream(
            &agent,
            &sub,
            version,
            line_max_bytes,
            &shutdown,
            |item| match item {
                StreamItem::Open => {
                    backoff.reset();
                    let _ = tx.send(AppEvent::SubscriptionConnected { name: name.clone() });
                }
                StreamItem::Keepalive | StreamItem::PollRequest => {}
                StreamItem::Message(event) => {
                    if let Some(notification) = filter.filter(event) {
                        let _ = tx.send(AppEvent::NtfyMessage {
                            subscription: name.clone(),
                            notification,
                        });
                    }
                }
                StreamItem::InvalidJson(err) => {
                    log::warn!("{} stream JSON line ignored: {err}", name);
                }
            },
        );

        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        match result {
            Ok(()) => {}
            Err(err) => {
                log::warn!("subscription {} disconnected: {}", sub.name, err.message);
                let _ = tx.send(AppEvent::SubscriptionDisconnected {
                    name: sub.name.clone(),
                    reason: err.message,
                });
                let delay = backoff.next_delay(err.slow_backoff);
                sleep_until_shutdown(delay, &shutdown);
            }
        }
    }
}

fn sleep_until_shutdown(delay: Duration, shutdown: &AtomicBool) {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline && !shutdown.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(200));
    }
}

fn join_with_timeout(handle: JoinHandle<()>, timeout: Duration) {
    let start = Instant::now();
    while !handle.is_finished() && start.elapsed() < timeout {
        thread::sleep(Duration::from_millis(20));
    }
    if handle.is_finished() {
        let _ = handle.join();
    }
}
