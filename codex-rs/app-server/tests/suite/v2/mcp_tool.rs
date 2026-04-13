use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use app_test_support::McpProcess;
use app_test_support::to_response;
use app_test_support::write_mock_responses_config_toml;
use axum::Router;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::McpServerToolCallParams;
use codex_app_server_protocol::McpServerToolCallResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use core_test_support::responses;
use pretty_assertions::assert_eq;
use rmcp::handler::server::ServerHandler;
use rmcp::model::CallToolRequestParams;
use rmcp::model::CallToolResult;
use rmcp::model::Content;
use rmcp::model::CustomRequest;
use rmcp::model::CustomResult;
use rmcp::model::ExperimentalCapabilities;
use rmcp::model::JsonObject;
use rmcp::model::ListToolsResult;
use rmcp::model::Meta;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;
use rmcp::model::ToolAnnotations;
use rmcp::service::RequestContext;
use rmcp::service::RoleServer;
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use serde_json::json;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(10);
const TEST_SERVER_NAME: &str = "tool_server";
const TEST_TOOL_NAME: &str = "echo_tool";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_server_tool_call_returns_tool_result() -> Result<()> {
    let responses_server = responses::start_mock_server().await;
    let (mcp_server_url, mcp_server_handle) = start_mcp_server().await?;
    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml(
        codex_home.path(),
        &responses_server.uri(),
        &BTreeMap::new(),
        /*auto_compact_limit*/ 1024,
        /*requires_openai_auth*/ None,
        "mock_provider",
        "compact",
    )?;

    let config_path = codex_home.path().join("config.toml");
    let mut config_toml = std::fs::read_to_string(&config_path)?;
    config_toml.push_str(&format!(
        r#"
[mcp_servers.{TEST_SERVER_NAME}]
url = "{mcp_server_url}/mcp"
"#
    ));
    std::fs::write(config_path, config_toml)?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response(thread_start_resp)?;

    let tool_call_request_id = mcp
        .send_mcp_server_tool_call_request(McpServerToolCallParams {
            thread_id: thread.id,
            server: TEST_SERVER_NAME.to_string(),
            tool: TEST_TOOL_NAME.to_string(),
            arguments: Some(json!({
                "message": "hello from app",
            })),
            meta: Some(json!({
                "source": "mcp-app",
            })),
        })
        .await?;
    let tool_call_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(tool_call_request_id)),
    )
    .await??;
    let response: McpServerToolCallResponse = to_response(tool_call_response)?;

    assert_eq!(response.content.len(), 1);
    assert_eq!(response.content[0].get("type"), Some(&json!("text")));
    assert_eq!(
        response.content[0].get("text"),
        Some(&json!("echo: hello from app"))
    );
    assert_eq!(
        response.structured_content,
        Some(json!({
            "echoed": "hello from app",
        }))
    );
    assert_eq!(response.is_error, Some(false));
    assert_eq!(
        response.meta,
        Some(json!({
            "calledBy": "mcp-app",
        }))
    );

    mcp_server_handle.abort();
    let _ = mcp_server_handle.await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_server_tool_call_includes_sandbox_state_meta_for_supporting_servers() -> Result<()> {
    let responses_server = responses::start_mock_server().await;
    let (mcp_server_url, mcp_server_handle, sandbox_state_updates) =
        start_meta_echo_mcp_server().await?;
    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml(
        codex_home.path(),
        &responses_server.uri(),
        &BTreeMap::new(),
        /*auto_compact_limit*/ 1024,
        /*requires_openai_auth*/ None,
        "mock_provider",
        "compact",
    )?;

    let config_path = codex_home.path().join("config.toml");
    let mut config_toml = std::fs::read_to_string(&config_path)?;
    config_toml.push_str(&format!(
        r#"
[mcp_servers.{TEST_SERVER_NAME}]
url = "{mcp_server_url}/mcp"
"#
    ));
    std::fs::write(config_path, config_toml)?;

    let sandbox_cwd = TempDir::new()?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            cwd: Some(sandbox_cwd.path().display().to_string()),
            sandbox: Some(codex_app_server_protocol::SandboxMode::ReadOnly),
            ..Default::default()
        })
        .await?;
    let thread_start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response(thread_start_resp)?;

    let tool_call_request_id = mcp
        .send_mcp_server_tool_call_request(McpServerToolCallParams {
            thread_id: thread.id,
            server: TEST_SERVER_NAME.to_string(),
            tool: TEST_TOOL_NAME.to_string(),
            arguments: Some(json!({
                "message": "hello from app",
            })),
            meta: Some(json!({
                "source": "mcp-app",
            })),
        })
        .await?;
    let tool_call_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(tool_call_request_id)),
    )
    .await??;
    let response: McpServerToolCallResponse = to_response(tool_call_response)?;

    let request_meta = response
        .structured_content
        .as_ref()
        .and_then(|value| value.get("requestMeta"))
        .cloned()
        .expect("requestMeta should be echoed");
    assert_eq!(request_meta["source"], json!("mcp-app"));
    assert_eq!(
        request_meta["codex/sandbox-state-meta"]["sandboxPolicy"]["type"],
        json!("read-only")
    );
    assert_eq!(
        request_meta["codex/sandbox-state-meta"]["sandboxCwd"],
        json!(sandbox_cwd.path())
    );
    assert_eq!(
        request_meta["codex/sandbox-state-meta"]["useLegacyLandlock"],
        json!(false)
    );
    let sandbox_state_updates = sandbox_state_updates
        .lock()
        .expect("sandbox state updates lock");
    assert_eq!(sandbox_state_updates.len(), 1);
    assert_eq!(
        sandbox_state_updates[0]["sandboxPolicy"]["type"],
        json!("read-only")
    );
    assert_eq!(
        sandbox_state_updates[0]["sandboxCwd"],
        json!(sandbox_cwd.path())
    );
    assert_eq!(sandbox_state_updates[0]["useLegacyLandlock"], json!(false));

    mcp_server_handle.abort();
    let _ = mcp_server_handle.await;

    Ok(())
}

#[tokio::test]
async fn mcp_server_tool_call_returns_error_for_unknown_thread() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_mcp_server_tool_call_request(McpServerToolCallParams {
            thread_id: "00000000-0000-4000-8000-000000000000".to_string(),
            server: TEST_SERVER_NAME.to_string(),
            tool: TEST_TOOL_NAME.to_string(),
            arguments: Some(json!({})),
            meta: None,
        })
        .await?;
    let error: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert!(
        error.error.message.contains("thread not found"),
        "expected thread-not-found error, got: {error:?}"
    );

    Ok(())
}

#[derive(Clone, Default)]
struct ToolAppsMcpServer;

impl ServerHandler for ToolAppsMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..ServerInfo::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let input_schema: JsonObject = serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string"
                }
            },
            "additionalProperties": false
        }))
        .map_err(|err| rmcp::ErrorData::internal_error(err.to_string(), None))?;

        let mut tool = Tool::new(
            Cow::Borrowed(TEST_TOOL_NAME),
            Cow::Borrowed("Echo a message."),
            Arc::new(input_schema),
        );
        tool.annotations = Some(ToolAnnotations::new().read_only(true));

        Ok(ListToolsResult {
            tools: vec![tool],
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        assert_eq!(request.name.as_ref(), TEST_TOOL_NAME);
        let message = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("message"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        let mut meta = Meta::new();
        meta.0.insert("calledBy".to_string(), json!("mcp-app"));

        let mut result = CallToolResult::structured(json!({
            "echoed": message,
        }));
        result.content = vec![Content::text(format!("echo: {message}"))];
        result.meta = Some(meta);
        Ok(result)
    }
}

async fn start_mcp_server() -> Result<(String, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let mcp_service = StreamableHttpService::new(
        || Ok(ToolAppsMcpServer),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = Router::new().nest_service("/mcp", mcp_service);

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    Ok((format!("http://{addr}"), handle))
}

#[derive(Clone, Default)]
struct MetaEchoMcpServer {
    sandbox_state_updates: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl ServerHandler for MetaEchoMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut experimental = ExperimentalCapabilities::new();
        experimental.insert("codex/sandbox-state".to_string(), JsonObject::new());
        experimental.insert("codex/sandbox-state-meta".to_string(), JsonObject::new());

        ServerInfo {
            capabilities: ServerCapabilities::builder()
                .enable_experimental_with(experimental)
                .enable_tools()
                .build(),
            ..ServerInfo::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let input_schema: JsonObject = serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string"
                }
            },
            "additionalProperties": false
        }))
        .map_err(|err| rmcp::ErrorData::internal_error(err.to_string(), None))?;

        Ok(ListToolsResult {
            tools: vec![Tool::new(
                Cow::Borrowed(TEST_TOOL_NAME),
                Cow::Borrowed("Echo a message."),
                Arc::new(input_schema),
            )],
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        assert_eq!(request.name.as_ref(), TEST_TOOL_NAME);
        let mut request_meta = context.meta.0.clone();
        if let Some(meta) = request.meta {
            request_meta.extend(meta.0);
        }
        Ok(CallToolResult::structured(json!({
            "requestMeta": request_meta,
        })))
    }

    async fn on_custom_request(
        &self,
        request: CustomRequest,
        _context: RequestContext<RoleServer>,
    ) -> Result<CustomResult, rmcp::ErrorData> {
        assert_eq!(request.method, "codex/sandbox-state/update");
        self.sandbox_state_updates
            .lock()
            .expect("sandbox state updates lock")
            .push(request.params.unwrap_or(serde_json::Value::Null));
        Ok(CustomResult(serde_json::Value::Null))
    }
}

async fn start_meta_echo_mcp_server()
-> Result<(String, JoinHandle<()>, Arc<Mutex<Vec<serde_json::Value>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let sandbox_state_updates = Arc::new(Mutex::new(Vec::new()));
    let sandbox_state_updates_for_server = Arc::clone(&sandbox_state_updates);
    let mcp_service = StreamableHttpService::new(
        move || {
            Ok(MetaEchoMcpServer {
                sandbox_state_updates: Arc::clone(&sandbox_state_updates_for_server),
            })
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = Router::new().nest_service("/mcp", mcp_service);

    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    Ok((format!("http://{addr}"), handle, sandbox_state_updates))
}
