use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    app::{AppEvent, EventSender},
    config::{Config, NetworkConfig, SubscriptionConfig},
    ntfy::{
        client::{self, StreamItem},
        filter::MessageFilter,
    },
    worker::reconnect::Backoff,
};

const WORKER_STACK_BYTES: usize = 256 * 1024;

struct WorkerNetwork {
    reconnect_initial_seconds: u64,
    reconnect_max_seconds: u64,
    reconnect_jitter: bool,
    line_max_bytes: usize,
}

impl From<&NetworkConfig> for WorkerNetwork {
    fn from(network: &NetworkConfig) -> Self {
        Self {
            reconnect_initial_seconds: network.reconnect_initial_seconds,
            reconnect_max_seconds: network.reconnect_max_seconds,
            reconnect_jitter: network.reconnect_jitter,
            line_max_bytes: network.line_max_bytes,
        }
    }
}

pub struct WorkerGroup {
    shutdown: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl WorkerGroup {
    pub fn start(config: &Config, tx: EventSender) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        let http = match client::agent(config.security.skip_tls_verify) {
            Ok(http) => Arc::new(http),
            Err(err) => {
                log::error!("cannot initialize shared HTTP client: {err}");
                for sub in &config.subscriptions {
                    let _ = tx.send(AppEvent::SubscriptionDisconnected {
                        name: sub.name.clone(),
                        reason: err.clone(),
                    });
                }
                return Self { shutdown, handles };
            }
        };

        let notification = Arc::new(config.notification.clone());
        let security = Arc::new(config.security.clone());
        let network = Arc::new(WorkerNetwork::from(&config.network));

        for sub in &config.subscriptions {
            let tx = tx.clone();
            let http = http.clone();
            let worker_shutdown = shutdown.clone();
            let notification = notification.clone();
            let security = security.clone();
            let network = network.clone();
            let sub = sub.clone();
            let handle = thread::Builder::new()
                .name(format!("ntfy-{}", sub.name))
                .stack_size(WORKER_STACK_BYTES)
                .spawn(move || {
                    run_worker(
                        http,
                        sub,
                        notification,
                        security,
                        network,
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
        for handle in &self.handles {
            handle.thread().unpark();
        }
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
    http: Arc<client::HttpClient>,
    sub: SubscriptionConfig,
    notification: Arc<crate::config::NotificationConfig>,
    security: Arc<crate::config::SecurityConfig>,
    network: Arc<WorkerNetwork>,
    tx: EventSender,
    shutdown: Arc<AtomicBool>,
) {
    let name = sub.name.clone();
    let mut backoff = Backoff::new(
        network.reconnect_initial_seconds,
        network.reconnect_max_seconds,
        network.reconnect_jitter,
    );
    let mut filter = MessageFilter::from_shared(notification, security);

    while !shutdown.load(Ordering::Relaxed) {
        let result = client::read_stream(
            &http,
            &sub,
            client::user_agent(),
            network.line_max_bytes,
            &shutdown,
            |item| match item {
                StreamItem::Open => {
                    backoff.reset();
                    let _ = tx.send(AppEvent::SubscriptionConnected { name: name.clone() });
                }
                StreamItem::Keepalive | StreamItem::PollRequest => {}
                StreamItem::Message(event) => {
                    if let Some(notification) = filter.filter(event) {
                        let _ = tx.try_send_notification(name.clone(), notification);
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
                log::warn!("subscription {} disconnected: {}", name, err.message);
                let _ = tx.send(AppEvent::SubscriptionDisconnected {
                    name: name.clone(),
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
        thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
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
