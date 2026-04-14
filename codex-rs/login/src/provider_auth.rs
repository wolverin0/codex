use std::sync::Arc;

use codex_model_provider::ProviderAuthKind;
use codex_model_provider::ProviderRuntime;
use codex_model_provider_info::ModelProviderInfo;

use crate::AuthManager;

/// Returns the provider-scoped auth manager when this provider uses command-backed auth.
///
/// Providers without custom auth continue using the caller-supplied base manager.
pub fn auth_manager_for_provider(
    auth_manager: Option<Arc<AuthManager>>,
    provider: &ModelProviderInfo,
) -> Option<Arc<AuthManager>> {
    match provider.auth.clone() {
        Some(config) => Some(AuthManager::external_bearer_only(config)),
        None => auth_manager,
    }
}

pub fn auth_manager_for_provider_runtime(
    auth_manager: Option<Arc<AuthManager>>,
    provider_runtime: &ProviderRuntime,
    legacy_provider: &ModelProviderInfo,
) -> Option<Arc<AuthManager>> {
    match provider_runtime {
        ProviderRuntime::Legacy => auth_manager_for_provider(auth_manager, legacy_provider),
        ProviderRuntime::Resolved(provider) => {
            auth_manager_for_provider_auth(auth_manager, &provider.auth)
        }
    }
}

pub fn auth_manager_for_provider_auth(
    auth_manager: Option<Arc<AuthManager>>,
    provider_auth: &ProviderAuthKind,
) -> Option<Arc<AuthManager>> {
    match provider_auth {
        ProviderAuthKind::CommandBearer { config } => {
            Some(AuthManager::external_bearer_only(config.clone()))
        }
        ProviderAuthKind::EnvBearer { .. }
        | ProviderAuthKind::StaticBearer { .. }
        | ProviderAuthKind::AuthManager => auth_manager,
    }
}

/// Returns an auth manager for request paths that always require authentication.
///
/// Providers with command-backed auth get a bearer-only manager; otherwise the caller's manager
/// is reused unchanged.
pub fn required_auth_manager_for_provider(
    auth_manager: Arc<AuthManager>,
    provider: &ModelProviderInfo,
) -> Arc<AuthManager> {
    match provider.auth.clone() {
        Some(config) => AuthManager::external_bearer_only(config),
        None => auth_manager,
    }
}

#[cfg(test)]
mod tests {
    use codex_model_provider::ProviderResolutionPolicy;
    use codex_model_provider::resolve_model_provider;
    use codex_model_provider_info::WireApi;
    use codex_protocol::config_types::ModelProviderAuthInfo;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use std::num::NonZeroU64;

    use super::*;

    fn provider_with_command_auth() -> ModelProviderInfo {
        ModelProviderInfo {
            name: "custom".to_string(),
            base_url: Some("https://example.com/v1".to_string()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: Some(ModelProviderAuthInfo {
                command: "print-token".to_string(),
                args: Vec::new(),
                timeout_ms: NonZeroU64::MIN,
                refresh_interval_ms: 0,
                cwd: AbsolutePathBuf::resolve_path_against_base(".", "/tmp"),
            }),
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        }
    }

    #[test]
    fn runtime_auth_manager_adapter_uses_resolved_command_auth() {
        let provider = provider_with_command_auth();
        let mut runtime = resolve_model_provider(
            "custom",
            &provider,
            &ProviderResolutionPolicy::with_enabled_provider_ids(["custom".to_string()]),
        );
        let ProviderRuntime::Resolved(resolved) = &mut runtime else {
            panic!("enabled provider should resolve through the provider framework");
        };
        resolved.info.auth = None;

        assert!(auth_manager_for_provider(None, &provider).is_some());
        assert!(auth_manager_for_provider_runtime(None, &runtime, &provider).is_some());
    }

    #[test]
    fn runtime_auth_manager_adapter_preserves_absent_auth_manager_for_plain_provider() {
        let provider = ModelProviderInfo {
            auth: None,
            ..provider_with_command_auth()
        };
        let runtime = resolve_model_provider(
            "custom",
            &provider,
            &ProviderResolutionPolicy::with_enabled_provider_ids(["custom".to_string()]),
        );

        assert!(auth_manager_for_provider(None, &provider).is_none());
        assert!(auth_manager_for_provider_runtime(None, &runtime, &provider).is_none());
    }
}
