//! Runtime provider abstraction for model-provider-specific behavior.
//!
//! The framework is intentionally opt-in. Providers continue through legacy
//! auth, transport, model-listing, and capability paths unless a resolution
//! policy explicitly enables the selected provider.

use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_protocol::openai_models::ModelsResponse;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;
use url::Url;

/// Runtime strategy selected for the active model provider.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderRuntime {
    /// Preserve the existing provider implementation.
    Legacy,
    /// Use provider-owned runtime strategies.
    Resolved(ResolvedModelProvider),
}

impl Default for ProviderRuntime {
    fn default() -> Self {
        Self::Legacy
    }
}

/// Runtime provider object resolved from config-facing provider metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModelProvider {
    pub id: String,
    pub info: ModelProviderInfo,
    pub auth: ProviderAuthKind,
    pub model_catalog: ProviderModelCatalog,
    pub transport: ProviderTransport,
    pub capabilities: ProviderCapabilities,
}

/// Provider-owned authentication strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProviderAuthKind {
    /// Preserve the existing provider-auth behavior for resolved test providers.
    Legacy,
    /// OpenAI-managed auth.
    OpenAi,
    /// Bearer token read from a configured environment variable.
    EnvBearer { env_key: String },
    /// Command-backed bearer token.
    CommandBearer,
    /// No provider auth is required.
    None,
}

/// Provider-owned model catalog source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProviderModelCatalog {
    /// Preserve the existing model-listing behavior.
    Legacy,
    /// Use a static catalog as authoritative.
    Static { models: ModelsResponse },
}

/// Provider-owned transport metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTransport {
    pub base_url: Option<Url>,
    pub wire_api: WireApi,
    pub request_timeout: Option<Duration>,
    pub supports_websockets: bool,
}

/// Provider-specific capability gates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub realtime: CapabilitySupport,
    pub audio_transcription: CapabilitySupport,
    pub image_generation: CapabilitySupport,
    pub web_search: CapabilitySupport,
    pub js_repl: CapabilitySupport,
    pub prompt_image_input: CapabilitySupport,
}

impl ProviderCapabilities {
    pub fn legacy_current_behavior() -> Self {
        Self {
            realtime: CapabilitySupport::Legacy,
            audio_transcription: CapabilitySupport::Legacy,
            image_generation: CapabilitySupport::Legacy,
            web_search: CapabilitySupport::Legacy,
            js_repl: CapabilitySupport::Legacy,
            prompt_image_input: CapabilitySupport::Legacy,
        }
    }
}

/// Whether a provider supports a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "support", rename_all = "camelCase")]
pub enum CapabilitySupport {
    /// Preserve existing feature/model-driven behavior.
    Legacy,
    /// Capability is supported by the provider.
    Supported,
    /// Capability is not supported by the provider.
    Unsupported { reason: String },
}

/// Policy controlling which providers may use the new runtime framework.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderResolutionPolicy {
    enabled_provider_ids: HashSet<String>,
}

impl ProviderResolutionPolicy {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn with_enabled_provider_ids(provider_ids: impl IntoIterator<Item = String>) -> Self {
        Self {
            enabled_provider_ids: provider_ids.into_iter().collect(),
        }
    }

    pub fn enables_provider(&self, provider_id: &str) -> bool {
        self.enabled_provider_ids.contains(provider_id)
    }
}

/// Resolve the config-facing provider into a runtime strategy.
///
/// This first PR intentionally keeps all providers on the legacy path. The
/// policy is accepted now so later PRs can add real opt-in resolution without
/// changing callsites again.
pub fn resolve_model_provider(
    _provider_id: &str,
    _provider: &ModelProviderInfo,
    _policy: &ProviderResolutionPolicy,
) -> ProviderRuntime {
    ProviderRuntime::Legacy
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
    use codex_model_provider_info::OLLAMA_OSS_PROVIDER_ID;
    use codex_model_provider_info::OPENAI_PROVIDER_ID;
    use codex_model_provider_info::WireApi;
    use codex_protocol::config_types::ModelProviderAuthInfo;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use std::num::NonZeroU64;

    #[test]
    fn empty_policy_keeps_known_providers_on_legacy_runtime() {
        let providers =
            codex_model_provider_info::built_in_model_providers(/*openai_base_url*/ None);
        let policy = ProviderResolutionPolicy::disabled();

        for provider_id in [
            OPENAI_PROVIDER_ID,
            OLLAMA_OSS_PROVIDER_ID,
            LMSTUDIO_OSS_PROVIDER_ID,
        ] {
            let provider = providers.get(provider_id).expect("provider should exist");
            assert_eq!(
                resolve_model_provider(provider_id, provider, &policy),
                ProviderRuntime::Legacy
            );
        }
    }

    #[test]
    fn empty_policy_keeps_custom_env_key_provider_on_legacy_runtime() {
        let provider = ModelProviderInfo {
            name: "custom".to_string(),
            base_url: Some("https://example.com/v1".to_string()),
            env_key: Some("CUSTOM_API_KEY".to_string()),
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

        assert_eq!(
            resolve_model_provider("custom", &provider, &ProviderResolutionPolicy::disabled()),
            ProviderRuntime::Legacy
        );
    }

    #[test]
    fn empty_policy_keeps_command_auth_provider_on_legacy_runtime() {
        let provider = ModelProviderInfo {
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
        };

        assert_eq!(
            resolve_model_provider("custom", &provider, &ProviderResolutionPolicy::disabled()),
            ProviderRuntime::Legacy
        );
    }

    #[test]
    fn enabled_policy_is_recorded_but_does_not_activate_framework_yet() {
        let providers =
            codex_model_provider_info::built_in_model_providers(/*openai_base_url*/ None);
        let provider = providers
            .get(OPENAI_PROVIDER_ID)
            .expect("provider should exist");
        let policy =
            ProviderResolutionPolicy::with_enabled_provider_ids([OPENAI_PROVIDER_ID.to_string()]);

        assert!(policy.enables_provider(OPENAI_PROVIDER_ID));
        assert_eq!(
            resolve_model_provider(OPENAI_PROVIDER_ID, provider, &policy),
            ProviderRuntime::Legacy
        );
    }
}
