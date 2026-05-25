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
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsOverrides;
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
use crate::tools::handlers::watch_spec::create_watch_tool;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;

const MIN_INTERVAL_SECS: u64 = 2;
const DEFAULT_INTERVAL_SECS: u64 = 10;
/// Cap the changed-output context delivered to the model so one watch tick
/// cannot blow the context window.
const MAX_OUTPUT_CHARS: usize = 4000;

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
            session, payload, ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "watch handler received unsupported payload".to_string(),
                ));
            }
        };

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

        spawn_command_watch(
            Arc::downgrade(&session),
            command.clone(),
            instruction.clone(),
            interval,
        );

        let msg = format!(
            "Watching `{command}` every {}s. When its output changes I will: {instruction}",
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
    command: String,
    instruction: String,
    interval: Duration,
) {
    tokio::spawn(async move {
        let mut last: Option<String> = None;
        loop {
            sleep(interval).await;
            let Some(session) = session.upgrade() else {
                break;
            };

            let output = match run_command(&command).await {
                Ok(output) => output,
                Err(error) => {
                    warn!("watch: command `{command}` failed: {error}");
                    continue;
                }
            };

            match last {
                // First poll establishes the baseline without firing.
                None => last = Some(output),
                // Unchanged: nothing to do.
                Some(ref prev) if *prev == output => {}
                // Changed: start a fresh turn so the agent reacts in this
                // session. submit_self works even when the agent is idle
                // (interactive + headless), unlike a trigger_turn mailbox which
                // only appends to an already-active turn.
                Some(_) => {
                    last = Some(output.clone());
                    let text = format!(
                        "{instruction}\n\n[watch fired — `{command}` output changed]\n{}",
                        truncate_chars(&output, MAX_OUTPUT_CHARS)
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
                }
            }
        }
    });
}

async fn run_command(command: &str) -> std::io::Result<String> {
    let output = Command::new("sh").arg("-c").arg(command).output().await?;
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(combined)
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
