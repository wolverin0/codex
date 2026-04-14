#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnifiedExecMonitorStatus {
    Starting,
    Running,
    Ready,
    Warning,
    Error,
}

impl UnifiedExecMonitorStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Ready => "READY",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UnifiedExecMonitorNotificationLevel {
    Info,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnifiedExecMonitorNotification {
    pub(crate) level: UnifiedExecMonitorNotificationLevel,
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnifiedExecMonitorUpdate {
    pub(crate) completed_lines: Vec<String>,
    pub(crate) notification: Option<UnifiedExecMonitorNotification>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnifiedExecMonitorState {
    status: UnifiedExecMonitorStatus,
    pending_line: String,
    ready_reported: bool,
    last_error_signature: Option<String>,
    last_warning_signature: Option<String>,
    last_event_summary: Option<String>,
}

impl Default for UnifiedExecMonitorState {
    fn default() -> Self {
        Self {
            status: UnifiedExecMonitorStatus::Starting,
            pending_line: String::new(),
            ready_reported: false,
            last_error_signature: None,
            last_warning_signature: None,
            last_event_summary: None,
        }
    }
}

impl UnifiedExecMonitorState {
    pub(crate) fn status(&self) -> UnifiedExecMonitorStatus {
        self.status
    }

    pub(crate) fn last_event_summary(&self) -> Option<&str> {
        self.last_event_summary.as_deref()
    }

    pub(crate) fn ingest_chunk(
        &mut self,
        chunk: &[u8],
        command_display: &str,
        process_id: Option<&str>,
    ) -> UnifiedExecMonitorUpdate {
        let text = String::from_utf8_lossy(chunk);
        let mut completed_lines = Vec::new();
        let mut notification = None;

        for ch in text.chars() {
            match ch {
                '\n' | '\r' => {
                    if let Some(line) = self.take_completed_line() {
                        if notification.is_none() {
                            notification = self.observe_line(&line, command_display, process_id);
                        }
                        completed_lines.push(line);
                    }
                }
                _ => self.pending_line.push(ch),
            }
        }

        if notification.is_none()
            && !completed_lines.is_empty()
            && self.status == UnifiedExecMonitorStatus::Starting
        {
            self.status = UnifiedExecMonitorStatus::Running;
        }

        UnifiedExecMonitorUpdate {
            completed_lines,
            notification,
        }
    }

    fn take_completed_line(&mut self) -> Option<String> {
        let line = self.pending_line.trim_end().trim().to_string();
        self.pending_line.clear();
        (!line.is_empty()).then_some(line)
    }

    fn observe_line(
        &mut self,
        line: &str,
        command_display: &str,
        process_id: Option<&str>,
    ) -> Option<UnifiedExecMonitorNotification> {
        let normalized = normalize_signature(line);

        if is_error_line(&normalized) {
            let should_notify = self.last_error_signature.as_deref() != Some(normalized.as_str());
            self.status = UnifiedExecMonitorStatus::Error;
            self.last_error_signature = Some(normalized);
            self.last_event_summary = Some(format!("error: {}", summarize_line(line)));
            if should_notify {
                return Some(UnifiedExecMonitorNotification {
                    level: UnifiedExecMonitorNotificationLevel::Error,
                    message: format_notification_message(
                        process_id,
                        command_display,
                        "reported an error",
                        line,
                    ),
                });
            }
            return None;
        }

        if is_ready_line(&normalized) {
            self.status = UnifiedExecMonitorStatus::Ready;
            self.last_event_summary = Some(format!("ready: {}", summarize_line(line)));
            if !self.ready_reported {
                self.ready_reported = true;
                return Some(UnifiedExecMonitorNotification {
                    level: UnifiedExecMonitorNotificationLevel::Info,
                    message: format_notification_message(
                        process_id,
                        command_display,
                        "is ready",
                        line,
                    ),
                });
            }
            return None;
        }

        if is_warning_line(&normalized) {
            let should_update = self.last_warning_signature.as_deref() != Some(normalized.as_str());
            self.last_warning_signature = Some(normalized);
            if should_update {
                self.status = UnifiedExecMonitorStatus::Warning;
                self.last_event_summary = Some(format!("warning: {}", summarize_line(line)));
            }
            return None;
        }

        if self.status == UnifiedExecMonitorStatus::Starting {
            self.status = UnifiedExecMonitorStatus::Running;
        }
        None
    }
}

fn format_notification_message(
    process_id: Option<&str>,
    command_display: &str,
    action: &str,
    line: &str,
) -> String {
    let process_label = process_id
        .map(|id| format!("background terminal {id}"))
        .unwrap_or_else(|| "background terminal".to_string());
    format!(
        "Monitor: {process_label} {action} for `{}` ({})",
        summarize_line(command_display),
        summarize_line(line)
    )
}

fn summarize_line(input: &str) -> String {
    let trimmed = input.trim();
    let mut out = String::new();
    let mut count = 0usize;
    for ch in trimmed.chars() {
        if count >= 120 {
            break;
        }
        out.push(ch);
        count += 1;
    }
    if trimmed.chars().count() > 120 {
        out.push_str("...");
    }
    out
}

fn normalize_signature(line: &str) -> String {
    line.trim().to_ascii_lowercase()
}

fn is_ready_line(line: &str) -> bool {
    [
        "compiled successfully",
        "ready in ",
        "ready on ",
        "ready at ",
        "server started",
        "listening on",
        "listening at",
        "running at",
        "available at",
        "local:",
        "http://localhost",
        "https://localhost",
        "started on port",
    ]
    .iter()
    .any(|pattern| line.contains(pattern))
}

fn is_error_line(line: &str) -> bool {
    if line.contains("0 errors") || line.contains("without errors") {
        return false;
    }

    line.starts_with("error")
        || line.contains(" error:")
        || line.contains(" failed")
        || line.contains("failed to compile")
        || line.contains("compilation failed")
        || line.contains("traceback")
        || line.contains("unhandled")
        || line.contains("exception")
        || line.contains("panic")
}

fn is_warning_line(line: &str) -> bool {
    line.starts_with("warning") || line.contains(" warning:")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::UnifiedExecMonitorNotificationLevel;
    use super::UnifiedExecMonitorState;
    use super::UnifiedExecMonitorStatus;

    #[test]
    fn detects_ready_once_and_tracks_status() {
        let mut state = UnifiedExecMonitorState::default();
        let update = state.ingest_chunk(b"VITE ready in 250ms\n", "npm run dev", Some("1234"));

        assert_eq!(state.status(), UnifiedExecMonitorStatus::Ready);
        assert_eq!(
            update.completed_lines,
            vec!["VITE ready in 250ms".to_string()]
        );
        let notification = update.notification.expect("expected ready notification");
        assert_eq!(
            notification.level,
            UnifiedExecMonitorNotificationLevel::Info
        );
        assert!(
            notification
                .message
                .contains("background terminal 1234 is ready")
        );

        let second = state.ingest_chunk(b"ready in 300ms\n", "npm run dev", Some("1234"));
        assert!(second.notification.is_none());
    }

    #[test]
    fn detects_error_once_per_signature() {
        let mut state = UnifiedExecMonitorState::default();
        let first = state.ingest_chunk(b"Error: failed to compile\n", "npm run dev", Some("1234"));
        assert_eq!(state.status(), UnifiedExecMonitorStatus::Error);
        assert!(first.notification.is_some());

        let second = state.ingest_chunk(b"Error: failed to compile\n", "npm run dev", Some("1234"));
        assert!(second.notification.is_none());
    }

    #[test]
    fn buffers_partial_lines_until_newline() {
        let mut state = UnifiedExecMonitorState::default();
        let first = state.ingest_chunk(b"listening", "python app.py", Some("99"));
        assert!(first.completed_lines.is_empty());
        assert!(first.notification.is_none());

        let second = state.ingest_chunk(b" on port 3000\n", "python app.py", Some("99"));
        assert_eq!(state.status(), UnifiedExecMonitorStatus::Ready);
        assert_eq!(
            second.completed_lines,
            vec!["listening on port 3000".to_string()]
        );
        assert!(second.notification.is_some());
    }
}
