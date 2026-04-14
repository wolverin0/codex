use codex_api::CoreAuthProvider;
use codex_model_provider::ProviderAuthKind;
use codex_model_provider::ProviderRuntime;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::error::CodexErr;
use codex_protocol::error::EnvVarError;

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
    match provider_runtime {
        ProviderRuntime::Legacy => auth_provider_from_auth(auth, legacy_provider),
        ProviderRuntime::Resolved(provider) => {
            auth_provider_from_provider_auth(auth, &provider.auth)
        }
    }
}

pub fn auth_provider_from_provider_auth(
    auth: Option<CodexAuth>,
    provider_auth: &ProviderAuthKind,
) -> codex_protocol::error::Result<CoreAuthProvider> {
    match provider_auth {
        ProviderAuthKind::EnvBearer {
            env_key,
            instructions,
        } => {
            let token = std::env::var(env_key)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    CodexErr::EnvVar(EnvVarError {
                        var: env_key.clone(),
                        instructions: instructions.clone(),
                    })
                })?;
            Ok(CoreAuthProvider {
                token: Some(token),
                account_id: None,
            })
        }
        ProviderAuthKind::StaticBearer { token } => Ok(CoreAuthProvider {
            token: Some(token.clone()),
            account_id: None,
        }),
        ProviderAuthKind::CommandBearer { .. } | ProviderAuthKind::AuthManager => {
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
    }
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
    fn runtime_auth_adapter_uses_resolved_static_bearer_auth() {
        let provider = bearer_provider();
        let mut runtime = resolve_model_provider(
            "custom",
            &provider,
            &ProviderResolutionPolicy::with_enabled_provider_ids(["custom".to_string()]),
        );
        let ProviderRuntime::Resolved(resolved) = &mut runtime else {
            panic!("enabled provider should resolve through the provider framework");
        };
        resolved.info.experimental_bearer_token = None;

        let legacy = auth_provider_from_auth(None, &provider).expect("legacy auth");
        let resolved = auth_provider_from_runtime(None, &runtime, &provider).expect("runtime auth");

        assert_eq!(resolved.bearer_token(), legacy.bearer_token());
        assert_eq!(resolved.account_id(), legacy.account_id());
    }

    #[test]
    fn runtime_auth_adapter_uses_resolved_env_key_errors() {
        let provider = missing_env_key_provider();
        let mut runtime = resolve_model_provider(
            "custom",
            &provider,
            &ProviderResolutionPolicy::with_enabled_provider_ids(["custom".to_string()]),
        );
        let ProviderRuntime::Resolved(resolved) = &mut runtime else {
            panic!("enabled provider should resolve through the provider framework");
        };
        resolved.info.env_key = None;
        resolved.info.env_key_instructions = None;

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

    #[test]
    fn runtime_auth_adapter_uses_auth_manager_fallback() {
        let provider = ModelProviderInfo {
            experimental_bearer_token: None,
            requires_openai_auth: false,
            ..bearer_provider()
        };
        let runtime = resolve_model_provider(
            "custom",
            &provider,
            &ProviderResolutionPolicy::with_enabled_provider_ids(["custom".to_string()]),
        );
        let auth = Some(CodexAuth::from_api_key("auth-manager-token"));

        let legacy = auth_provider_from_auth(auth.clone(), &provider).expect("legacy auth");
        let resolved = auth_provider_from_runtime(auth, &runtime, &provider).expect("runtime auth");

        assert_eq!(resolved.bearer_token(), legacy.bearer_token());
        assert_eq!(resolved.account_id(), legacy.account_id());
    }
}
