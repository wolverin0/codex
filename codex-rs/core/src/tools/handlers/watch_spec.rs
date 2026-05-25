use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

pub const WATCH_TOOL_NAME: &str = "watch";

/// Build the model-callable `watch` tool spec.
///
/// This is the agent-facing equivalent of Claude Code's Monitor tool: the model
/// calls it from natural language ("watch pane 0 and react when it changes"),
/// the runtime polls `command` in the background, and when its output changes it
/// starts a new turn carrying `instruction` so the agent reacts on its own.
pub fn create_watch_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "command".to_string(),
            JsonSchema::string(Some(
                "Shell command to poll for changes. Its stdout is captured each interval; when the \
                 output differs from the previous poll, the watch fires. Examples: `cat roadmap.md`, \
                 a command that prints a pane's text, `tail -n 20 build.log`."
                    .to_string(),
            )),
        ),
        (
            "instruction".to_string(),
            JsonSchema::string(Some(
                "What you (the agent) should do when the watched command's output changes. This \
                 text is delivered to you as a new user turn when the watch fires."
                    .to_string(),
            )),
        ),
        (
            "interval_seconds".to_string(),
            JsonSchema::number(Some(
                "How often to poll the command, in seconds. Defaults to 10. Minimum 2.".to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: WATCH_TOOL_NAME.to_string(),
        description: "Watch a source for changes and react automatically. Polls `command` in the \
                      background; when its stdout changes, you receive a new turn containing \
                      `instruction` and the changed output, so you can act without the user asking \
                      again. Use this when the user says things like \"watch X and do Y when it \
                      changes\". The watch runs until the session ends."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["command".to_string(), "instruction".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
