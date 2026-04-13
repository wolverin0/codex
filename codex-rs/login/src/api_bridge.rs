use codex_api::CoreAuthProvider;
use codex_model_provider::ProviderRuntime;
use codex_model_provider_info::ModelProviderInfo;

use crate::CodexAuth;

pub fn auth_provider_from_auth(
    auth: Option<CodexAuth>,
    provider: &ModelProviderInfo,
) -> codex_protocol::error::Result<CoreAuthProvider> {
    if let Some(api_key) = provider.api_key()? {
        return Ok(CoreAuthProvider {
            token: Some(api_key),
            account_id: None,
        });
    }

    if let Some(token) = provider.experimental_bearer_token.clone() {
        return Ok(CoreAuthProvider {
            token: Some(token),
            account_id: None,
        });
    }

    if let Some(auth) = auth {
        let token = auth.get_token()?;
        Ok(CoreAuthProvider {
            token: Some(token),
            account_id: auth.get_account_id(),
        })
    } else {
        Ok(CoreAuthProvider {
            token: None,
            account_id: None,
        })
    }
}

pub fn auth_provider_from_runtime(
    auth: Option<CodexAuth>,
    provider_runtime: &ProviderRuntime,
    legacy_provider: &ModelProviderInfo,
) -> codex_protocol::error::Result<CoreAuthProvider> {
    let provider = match provider_runtime {
        ProviderRuntime::Legacy => legacy_provider,
        ProviderRuntime::Resolved(provider) => &provider.info,
    };
    auth_provider_from_auth(auth, provider)
}

#[cfg(test)]
mod tests {
    use codex_api::AuthProvider;
    use codex_model_provider::ProviderResolutionPolicy;
    use codex_model_provider::resolve_model_provider;
    use codex_model_provider_info::ModelProviderInfo;
    use codex_model_provider_info::WireApi;
    use pretty_assertions::assert_eq;

    use super::*;

    fn bearer_provider() -> ModelProviderInfo {
        ModelProviderInfo {
            name: "custom".to_string(),
            base_url: Some("https://example.com/v1".to_string()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: Some("token".to_string()),
            auth: None,
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

    fn missing_env_key_provider() -> ModelProviderInfo {
        ModelProviderInfo {
            name: "custom".to_string(),
            base_url: Some("https://example.com/v1".to_string()),
            env_key: Some("MISSING_CODEX_PROVIDER_TEST_KEY".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
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
    fn runtime_auth_adapter_delegates_bearer_auth_to_legacy_provider() {
        let provider = bearer_provider();
        let runtime = resolve_model_provider(
            "custom",
            &provider,
            &ProviderResolutionPolicy::with_enabled_provider_ids(["custom".to_string()]),
        );

        let legacy = auth_provider_from_auth(None, &provider).expect("legacy auth");
        let resolved = auth_provider_from_runtime(None, &runtime, &provider).expect("runtime auth");

        assert_eq!(resolved.bearer_token(), legacy.bearer_token());
        assert_eq!(resolved.account_id(), legacy.account_id());
    }

    #[test]
    fn runtime_auth_adapter_delegates_env_key_errors_to_legacy_provider() {
        let provider = missing_env_key_provider();
        let runtime = resolve_model_provider(
            "custom",
            &provider,
            &ProviderResolutionPolicy::with_enabled_provider_ids(["custom".to_string()]),
        );

        let legacy = match auth_provider_from_auth(None, &provider) {
            Ok(_) => panic!("missing env key should fail"),
            Err(err) => err.to_string(),
        };
        let resolved = match auth_provider_from_runtime(None, &runtime, &provider) {
            Ok(_) => panic!("missing env key should fail"),
            Err(err) => err.to_string(),
        };

        assert_eq!(resolved, legacy);
    }
}
