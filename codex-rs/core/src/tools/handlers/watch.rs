//! Model-callable `watch` tool: the agent-facing equivalent of Claude Code's
//! Monitor tool.
//!
//! The model calls this from natural language ("watch pane 0 and react when it
//! changes"). The handler registers a background poll of `command`; when the
//! command's stdout changes, it submits a new `Op::UserInput` turn carrying the
//! standing `instruction`, so the agent reacts on its own — no human typing and
//! no slash command. Works in both the interactive TUI and headless `codex
//! exec`, because the reaction is started at the core session level.

use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use tokio::time::sleep;
use tracing::warn;

use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::protocol::WarningEvent;
use codex_protocol::user_input::UserInput;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::watch_spec::WATCH_TOOL_NAME;
use crate::tools::handlers::watch_spec::create_watch_list_tool;
use crate::tools::handlers::watch_spec::create_watch_stop_tool;
use crate::tools::handlers::watch_spec::create_watch_tool;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;

const MIN_INTERVAL_SECS: u64 = 2;
const DEFAULT_INTERVAL_SECS: u64 = 10;
/// Cap the changed-output context delivered to the model so one watch tick
/// cannot blow the context window.
const MAX_OUTPUT_CHARS: usize = 4000;
/// Maximum concurrent watches per session.
const MAX_WATCHES: usize = 8;
/// Auto-stop a watch after this many consecutive command failures.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

#[derive(Default)]
pub struct WatchHandler;

#[derive(Deserialize)]
struct WatchArgs {
    command: String,
    instruction: String,
    #[serde(default)]
    interval_seconds: Option<u64>,
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WatchHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(WATCH_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_watch_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "watch handler received unsupported payload".to_string(),
                ));
            }
        };

        if session.watch_count() >= MAX_WATCHES {
            return Err(FunctionCallError::RespondToModel(format!(
                "watch limit reached ({MAX_WATCHES} active); stop one with watch_stop first."
            )));
        }

        let WatchArgs {
            command,
            instruction,
            interval_seconds,
        } = parse_arguments(&arguments)?;

        if command.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "watch.command must not be empty".to_string(),
            ));
        }
        if instruction.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "watch.instruction must not be empty".to_string(),
            ));
        }

        let interval = Duration::from_secs(
            interval_seconds
                .unwrap_or(DEFAULT_INTERVAL_SECS)
                .max(MIN_INTERVAL_SECS),
        );

        // Confine the background poll to the turn's working directory.
        let cwd = turn.cwd.as_path().to_path_buf();
        // Pre-allocate the id so the task can self-deregister on auto-stop.
        let id = session.allocate_watch_id();
        let abort = spawn_command_watch(
            Arc::downgrade(&session),
            id,
            command.clone(),
            instruction.clone(),
            interval,
            cwd,
        );
        session.register_watch_with_id(id, command.clone(), instruction.clone(), abort);
        session
            .send_event_raw(Event {
                id: String::new(),
                msg: EventMsg::Warning(WarningEvent {
                    message: format!(
                        "👁 watch [{id}] active: `{command}` — {} watch(es) running",
                        session.watch_count()
                    ),
                }),
            })
            .await;

        let msg = format!(
            "Watching `{command}` (id {id}) every {}s. When its output changes I will: {instruction}",
            interval.as_secs()
        );
        Ok(boxed_tool_output(FunctionToolOutput::from_content(
            vec![FunctionCallOutputContentItem::InputText { text: msg }],
            Some(true),
        )))
    }
}

impl CoreToolRuntime for WatchHandler {}

/// Background poll loop. Holds a `Weak<Session>` so a registered watch never
/// keeps the session alive; the loop exits once the session is dropped.
fn spawn_command_watch(
    session: Weak<Session>,
    id: u64,
    command: String,
    instruction: String,
    interval: Duration,
    cwd: std::path::PathBuf,
) -> tokio::task::AbortHandle {
    let join = tokio::spawn(async move {
        // `prev_poll` is the previous poll's output (settle detection);
        // `last_fired` is the output we last reacted to. We only react once the
        // output has stopped changing for a full interval AND differs from the
        // last reaction — collapsing a burst (e.g. a streaming pane that updates
        // on every token, or a multi-write save) into a SINGLE reaction.
        let mut prev_poll: Option<String> = None;
        let mut last_fired: Option<String> = None;
        let mut consecutive_failures: u32 = 0;
        loop {
            sleep(interval).await;
            let Some(session) = session.upgrade() else {
                break;
            };

            let (ok, out) = match run_command(&command, &cwd).await {
                Ok(pair) => pair,
                Err(error) => (false, error.to_string()),
            };
            if !ok {
                consecutive_failures += 1;
                warn!(
                    "watch: command `{command}` failed \
                     ({consecutive_failures}/{MAX_CONSECUTIVE_FAILURES}): {}",
                    truncate_chars(&out, 200)
                );
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    // Emit the auto-stop notice through submit_self (a synthetic
                    // user turn) instead of send_event_raw(Warning). Background
                    // Warning events do NOT render in the TUI while the agent is
                    // idle, but submit_self turns do (same path as reactions).
                    let notice = format!(
                        "[watch auto-stopped after {MAX_CONSECUTIVE_FAILURES} \
                         consecutive failures: `{command}`] No further action \
                         needed; this watch will no longer poll."
                    );
                    let op = Op::UserInput {
                        items: vec![UserInput::Text {
                            text: notice,
                            text_elements: Vec::new(),
                        }],
                        environments: None,
                        final_output_json_schema: None,
                        responsesapi_client_metadata: None,
                        thread_settings: ThreadSettingsOverrides::default(),
                    };
                    let _ = session.submit_self(op).await;
                    // Self-deregister so the slot is freed and watch_list no
                    // longer claims this watch is active.
                    session.deregister_watch(id);
                    break;
                }
                continue;
            }
            consecutive_failures = 0;
            let output = out;

            // Require two consecutive identical polls (the source has quiesced)
            // before considering a reaction.
            let settled = prev_poll.as_deref() == Some(output.as_str());
            prev_poll = Some(output.clone());
            if !settled {
                continue;
            }

            match last_fired {
                // First settled state is the baseline; do not react to it.
                None => last_fired = Some(output),
                // Settled but unchanged since the last reaction.
                Some(ref prev) if *prev == output => {}
                // Settled into a genuinely new state: react once. Deliver only
                // the DIFF (what changed) instead of the whole capture, via
                // submit_self (starts a fresh turn even when the agent is idle).
                Some(ref prev) => {
                    let diff = diff_lines(prev, &output);
                    last_fired = Some(output.clone());
                    let text = format!(
                        "{instruction}\n\n[watch fired — `{command}` output changed]\n{}",
                        truncate_chars(&diff, MAX_OUTPUT_CHARS)
                    );
                    let op = Op::UserInput {
                        items: vec![UserInput::Text {
                            text,
                            text_elements: Vec::new(),
                        }],
                        environments: None,
                        final_output_json_schema: None,
                        responsesapi_client_metadata: None,
                        thread_settings: ThreadSettingsOverrides::default(),
                    };
                    if !session.submit_self(op).await {
                        warn!("watch: self-submit sender unavailable; reaction dropped");
                    }
                    // Cooldown: give the reaction room to run before polling
                    // again, so reactions don't stack while one is in flight.
                    sleep(interval * 3).await;
                }
            }
        }
    });
    join.abort_handle()
}

/// Run the poll command in the watch's working directory. Returns
/// `(success, combined stdout+stderr)`. A non-zero exit reports `false` so the
/// caller can count it toward auto-stop.
async fn run_command(command: &str, cwd: &std::path::Path) -> std::io::Result<(bool, String)> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()
        .await?;
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), combined))
}

/// Compact line diff (added `+`/removed `-` lines) between two captures, so the
/// agent receives what CHANGED rather than the whole output. Falls back to the
/// new output when the change is only reordering/whitespace.
fn diff_lines(old: &str, new: &str) -> String {
    let old_lines: std::collections::HashSet<&str> = old.lines().collect();
    let new_lines: std::collections::HashSet<&str> = new.lines().collect();
    // Dedupe within each side while preserving first-seen order — repeated
    // identical lines (e.g. five identical log appends) print as ONE `+` row.
    let mut printed_add = std::collections::HashSet::new();
    let mut printed_rem = std::collections::HashSet::new();
    let mut out = String::new();
    for line in new.lines() {
        if !old_lines.contains(line) && printed_add.insert(line) {
            out.push_str("+ ");
            out.push_str(line);
            out.push('\n');
        }
    }
    for line in old.lines() {
        if !new_lines.contains(line) && printed_rem.insert(line) {
            out.push_str("- ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() { new.to_string() } else { out }
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push_str("…[truncated]");
        out
    }
}

/// `watch_list`: report the active watches so the user/agent can see what's
/// being monitored instead of monitoring blindly.
#[derive(Default)]
pub struct WatchListHandler;

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WatchListHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("watch_list")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_watch_list_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
        let watches = invocation.session.list_watches();
        let msg = if watches.is_empty() {
            "No active watches.".to_string()
        } else {
            let mut out = String::from("Active watches:");
            for (id, command, instruction) in watches {
                out.push_str(&format!("\n  [{id}] `{command}` -> {instruction}"));
            }
            out
        };
        Ok(boxed_tool_output(FunctionToolOutput::from_content(
            vec![FunctionCallOutputContentItem::InputText { text: msg }],
            Some(true),
        )))
    }
}

impl CoreToolRuntime for WatchListHandler {}

/// `watch_stop`: stop an active watch by id.
#[derive(Default)]
pub struct WatchStopHandler;

#[derive(Deserialize)]
struct WatchStopArgs {
    id: u64,
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WatchStopHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("watch_stop")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_watch_stop_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session, payload, ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "watch_stop received unsupported payload".to_string(),
                ));
            }
        };

        let WatchStopArgs { id } = parse_arguments(&arguments)?;
        let msg = if session.stop_watch(id) {
            session
                .send_event_raw(Event {
                    id: String::new(),
                    msg: EventMsg::Warning(WarningEvent {
                        message: format!(
                            "👁 watch [{id}] stopped — {} watch(es) running",
                            session.watch_count()
                        ),
                    }),
                })
                .await;
            format!("Stopped watch [{id}].")
        } else {
            format!("No active watch with id {id}.")
        };

        Ok(boxed_tool_output(FunctionToolOutput::from_content(
            vec![FunctionCallOutputContentItem::InputText { text: msg }],
            Some(true),
        )))
    }
}

impl CoreToolRuntime for WatchStopHandler {}
