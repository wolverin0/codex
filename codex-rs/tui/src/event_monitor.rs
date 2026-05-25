//! Reactive file monitors for the TUI: the in-session equivalent of Claude
//! Code's Monitor tool.
//!
//! A `/watch <path> <instruction>` command registers an [`EventMonitor`] that
//! watches `path`. When the path changes, the monitor pushes an
//! [`AppEvent::MonitorFired`] into the live app event loop, which submits the
//! standing `instruction` as a turn so the agent reacts inline — without the
//! user typing anything.
//!
//! The watcher subscribes to the *parent directory* (non-recursive) and filters
//! by file name, so it survives editors that save via write-temp-then-rename
//! (which would break a watch held directly on the file's inode).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use notify::Event;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::Watcher;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;

/// Collapse rapid bursts of file events (e.g. editor save + format) into one
/// reaction: after the first event, wait for this much quiet before firing.
#[cfg(not(test))]
const MONITOR_DEBOUNCE: Duration = Duration::from_secs(2);
#[cfg(test)]
const MONITOR_DEBOUNCE: Duration = Duration::from_millis(100);

/// A single active monitor: watches `path` and fires `instruction` on change.
struct EventMonitor {
    id: u64,
    path: PathBuf,
    instruction: String,
    // Held only to keep the OS watch alive for the monitor's lifetime; dropped
    // on `stop`.
    _watcher: RecommendedWatcher,
    task: JoinHandle<()>,
}

impl EventMonitor {
    fn start(
        id: u64,
        path: PathBuf,
        instruction: String,
        app_event_tx: AppEventSender,
    ) -> anyhow::Result<Self> {
        let dir = path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let target_name = path.file_name().map(|n| n.to_os_string());

        // The notify callback runs on notify's own thread; forward a tick into
        // tokio. `UnboundedSender::send` is sync, so this is safe off-runtime.
        let (tx, mut rx) = mpsc::unbounded_channel::<()>();
        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    let hit = match &target_name {
                        Some(name) => event.paths.iter().any(|p| p.file_name() == Some(name)),
                        None => true,
                    };
                    if hit {
                        let _ = tx.send(());
                    }
                }
            })
            .context("failed to create file watcher")?;
        watcher
            .watch(&dir, RecursiveMode::NonRecursive)
            .with_context(|| format!("failed to watch {}", dir.display()))?;

        let instruction_for_task = instruction.clone();
        let path_for_task = path.clone();
        // Slash commands always dispatch inside the TUI's Tokio runtime, so a
        // current handle is guaranteed here.
        let handle = Handle::try_current().context("no Tokio runtime for event monitor")?;
        let task = handle.spawn(async move {
            while rx.recv().await.is_some() {
                // Debounce: keep resetting the quiet window until the file stops
                // changing, then fire exactly once for the burst.
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(MONITOR_DEBOUNCE) => break,
                        next = rx.recv() => {
                            if next.is_none() {
                                return;
                            }
                        }
                    }
                }
                app_event_tx.send(AppEvent::MonitorFired {
                    instruction: instruction_for_task.clone(),
                    paths: vec![path_for_task.clone()],
                });
            }
        });

        Ok(Self {
            id,
            path,
            instruction,
            _watcher: watcher,
            task,
        })
    }

    fn stop(self) {
        self.task.abort();
    }
}

/// Owns the set of active monitors for a session.
#[derive(Default)]
pub(crate) struct EventMonitorRegistry {
    next_id: u64,
    monitors: Vec<EventMonitor>,
}

impl EventMonitorRegistry {
    /// Register a new monitor for `path`. Returns the assigned id.
    pub(crate) fn add(
        &mut self,
        path: PathBuf,
        instruction: String,
        app_event_tx: AppEventSender,
    ) -> anyhow::Result<u64> {
        let id = self.next_id;
        let monitor = EventMonitor::start(id, path, instruction, app_event_tx)?;
        self.next_id += 1;
        self.monitors.push(monitor);
        Ok(id)
    }

    /// Stop and remove a monitor by id. Returns true if one was removed.
    pub(crate) fn stop(&mut self, id: u64) -> bool {
        match self.monitors.iter().position(|m| m.id == id) {
            Some(pos) => {
                self.monitors.remove(pos).stop();
                true
            }
            None => false,
        }
    }

    /// `(id, watched path, instruction)` for each active monitor.
    pub(crate) fn list(&self) -> impl Iterator<Item = (u64, &PathBuf, &str)> {
        self.monitors
            .iter()
            .map(|m| (m.id, &m.path, m.instruction.as_str()))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;
    use tokio::time::timeout;

    /// A registered monitor fires `AppEvent::MonitorFired` carrying the standing
    /// instruction and the changed path when the watched file is modified —
    /// the mechanism that makes the agent react in-session.
    #[tokio::test]
    async fn fires_monitor_event_on_file_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("roadmap.md");
        std::fs::write(&file, "init\n").unwrap();

        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut registry = EventMonitorRegistry::default();
        let id = registry
            .add(file.clone(), "do the task".to_string(), AppEventSender::new(tx))
            .expect("monitor should start");
        assert_eq!(id, 0);
        assert!(!registry.is_empty());

        // Let the OS watch register before mutating the file.
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::write(&file, "changed\n").unwrap();

        let event = timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("watcher did not fire within timeout")
            .expect("event channel closed");
        match event {
            AppEvent::MonitorFired { instruction, paths } => {
                assert_eq!(instruction, "do the task");
                assert_eq!(
                    paths.iter().filter_map(|p| p.file_name()).next(),
                    file.file_name(),
                    "fired path should be the watched file"
                );
            }
            _ => panic!("expected AppEvent::MonitorFired"),
        }
    }

    /// `list` reflects active monitors; `stop` removes one and reports whether a
    /// monitor with that id existed.
    #[tokio::test]
    async fn list_and_stop_manage_monitors() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();

        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let mut registry = EventMonitorRegistry::default();
        let id = registry
            .add(file.clone(), "instr".to_string(), AppEventSender::new(tx))
            .expect("monitor should start");

        let listed: Vec<(u64, PathBuf, String)> = registry
            .list()
            .map(|(i, p, instr)| (i, p.clone(), instr.to_string()))
            .collect();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, id);
        assert_eq!(listed[0].2, "instr");

        assert!(registry.stop(id), "stopping an existing id returns true");
        assert!(registry.is_empty());
        assert!(!registry.stop(id), "stopping a removed id returns false");
        assert!(!registry.stop(999), "stopping an unknown id returns false");
    }

    /// Ids are assigned monotonically so `stop <id>` targets the right monitor.
    #[tokio::test]
    async fn ids_are_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("one.txt");
        let f2 = dir.path().join("two.txt");
        std::fs::write(&f1, "1").unwrap();
        std::fs::write(&f2, "2").unwrap();
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let mut registry = EventMonitorRegistry::default();
        let id1 = registry
            .add(f1, "a".to_string(), AppEventSender::new(tx.clone()))
            .unwrap();
        let id2 = registry.add(f2, "b".to_string(), AppEventSender::new(tx)).unwrap();
        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
    }
}
