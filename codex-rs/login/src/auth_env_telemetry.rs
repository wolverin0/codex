use codex_model_provider::ProviderAuthKind;
use codex_model_provider::ProviderRuntime;
use codex_model_provider_info::ModelProviderInfo;
use codex_otel::AuthEnvTelemetryMetadata;

use crate::CODEX_API_KEY_ENV_VAR;
use crate::OPENAI_API_KEY_ENV_VAR;
use crate::REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthEnvTelemetry {
    pub openai_api_key_env_present: bool,
    pub codex_api_key_env_present: bool,
    pub codex_api_key_env_enabled: bool,
    pub provider_env_key_name: Option<String>,
    pub provider_env_key_present: Option<bool>,
    pub refresh_token_url_override_present: bool,
}

impl AuthEnvTelemetry {
    pub fn to_otel_metadata(&self) -> AuthEnvTelemetryMetadata {
        AuthEnvTelemetryMetadata {
            openai_api_key_env_present: self.openai_api_key_env_present,
            codex_api_key_env_present: self.codex_api_key_env_present,
            codex_api_key_env_enabled: self.codex_api_key_env_enabled,
            provider_env_key_name: self.provider_env_key_name.clone(),
            provider_env_key_present: self.provider_env_key_present,
            refresh_token_url_override_present: self.refresh_token_url_override_present,
        }
    }
}

pub fn collect_auth_env_telemetry(
    provider: &ModelProviderInfo,
    codex_api_key_env_enabled: bool,
) -> AuthEnvTelemetry {
    let provider_env_key = provider.env_key.as_deref();
    collect_auth_env_telemetry_for_env_key(provider_env_key, codex_api_key_env_enabled)
}

pub fn collect_auth_env_telemetry_for_runtime(
    provider_runtime: &ProviderRuntime,
    legacy_provider: &ModelProviderInfo,
    codex_api_key_env_enabled: bool,
) -> AuthEnvTelemetry {
    match provider_runtime {
        ProviderRuntime::Legacy => {
            collect_auth_env_telemetry(legacy_provider, codex_api_key_env_enabled)
        }
        ProviderRuntime::Resolved(provider) => {
            collect_auth_env_telemetry_for_provider_auth(&provider.auth, codex_api_key_env_enabled)
        }
    }
}

pub fn collect_auth_env_telemetry_for_provider_auth(
    provider_auth: &ProviderAuthKind,
    codex_api_key_env_enabled: bool,
) -> AuthEnvTelemetry {
    let provider_env_key = match provider_auth {
        ProviderAuthKind::EnvBearer { env_key, .. } => Some(env_key.as_str()),
        ProviderAuthKind::OpenAi
        | ProviderAuthKind::StaticBearer { .. }
        | ProviderAuthKind::CommandBearer { .. }
        | ProviderAuthKind::None => None,
    };
    collect_auth_env_telemetry_for_env_key(provider_env_key, codex_api_key_env_enabled)
}

fn collect_auth_env_telemetry_for_env_key(
    provider_env_key: Option<&str>,
    codex_api_key_env_enabled: bool,
) -> AuthEnvTelemetry {
    AuthEnvTelemetry {
        openai_api_key_env_present: env_var_present(OPENAI_API_KEY_ENV_VAR),
        codex_api_key_env_present: env_var_present(CODEX_API_KEY_ENV_VAR),
        codex_api_key_env_enabled,
        provider_env_key_name: provider_env_key.map(|_| "configured".to_string()),
        provider_env_key_present: provider_env_key.map(env_var_present),
        refresh_token_url_override_present: env_var_present(REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR),
    }
}

fn env_var_present(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => !value.trim().is_empty(),
        Err(std::env::VarError::NotUnicode(_)) => true,
        Err(std::env::VarError::NotPresent) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_model_provider::ProviderResolutionPolicy;
    use codex_model_provider::ProviderRuntime;
    use codex_model_provider::resolve_model_provider;
    use codex_model_provider_info::WireApi;
    use pretty_assertions::assert_eq;

    #[test]
    fn collect_auth_env_telemetry_buckets_provider_env_key_name() {
        let provider = ModelProviderInfo {
            name: "Custom".to_string(),
            base_url: None,
            env_key: Some("sk-should-not-leak".to_string()),
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
        };

        let telemetry =
            collect_auth_env_telemetry(&provider, /*codex_api_key_env_enabled*/ false);

        assert_eq!(
            telemetry.provider_env_key_name,
            Some("configured".to_string())
        );
    }

    #[test]
    fn runtime_auth_env_telemetry_uses_resolved_provider_auth() {
        let provider = ModelProviderInfo {
            name: "Custom".to_string(),
            base_url: None,
            env_key: Some("PATH".to_string()),
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
        };
        let mut runtime = resolve_model_provider(
            "custom",
            &provider,
            &ProviderResolutionPolicy::with_enabled_provider_ids(["custom".to_string()]),
        );
        let ProviderRuntime::Resolved(resolved) = &mut runtime else {
            panic!("enabled provider should resolve through the provider framework");
        };
        resolved.info.env_key = None;

        let telemetry = collect_auth_env_telemetry_for_runtime(
            &runtime, &provider, /*codex_api_key_env_enabled*/ false,
        );

        assert_eq!(
            telemetry.provider_env_key_name,
            Some("configured".to_string())
        );
        assert_eq!(telemetry.provider_env_key_present, Some(true));
    }
}
