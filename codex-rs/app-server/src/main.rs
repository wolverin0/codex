use clap::Parser;
use codex_app_server::AppServerTransport;
use codex_app_server::AppServerWebsocketAuthArgs;
use codex_app_server::run_main_with_transport;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_core::config_loader::LoaderOverrides;
use codex_protocol::protocol::SessionSource;
use codex_utils_cli::CliConfigOverrides;
use std::path::PathBuf;

// Debug-only test hook: lets integration tests point the server at a temporary
// managed config file without writing to /etc.
const MANAGED_CONFIG_PATH_ENV_VAR: &str = "CODEX_APP_SERVER_MANAGED_CONFIG_PATH";

#[derive(Debug, Parser)]
struct AppServerArgs {
    /// Transport endpoint URL. Supported values: `stdio://` (default),
    /// `ws://IP:PORT`, `off`.
    #[arg(
        long = "listen",
        value_name = "URL",
        default_value = AppServerTransport::DEFAULT_LISTEN_URL
    )]
    listen: AppServerTransport,

    /// Session source used to derive product restrictions and metadata.
    #[arg(
        long = "session-source",
        value_name = "SOURCE",
        default_value = "vscode",
        value_parser = SessionSource::from_startup_arg
    )]
    session_source: SessionSource,

    #[command(flatten)]
    auth: AppServerWebsocketAuthArgs,
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        let args = AppServerArgs::parse();
        let loader_overrides = loader_overrides_from_debug_env();
        let transport = args.listen;
        let session_source = args.session_source;
        let auth = args.auth.try_into_settings()?;

        run_main_with_transport(
            arg0_paths,
            CliConfigOverrides::default(),
            loader_overrides,
            /*default_analytics_enabled*/ false,
            transport,
            session_source,
            auth,
        )
        .await?;
        Ok(())
    })
}

fn loader_overrides_from_debug_env() -> LoaderOverrides {
    let mut loader_overrides = LoaderOverrides::default();

    #[cfg(debug_assertions)]
    {
        if let Ok(value) = std::env::var(MANAGED_CONFIG_PATH_ENV_VAR) {
            let managed_config_path = if value.is_empty() {
                std::env::temp_dir()
                    .join("codex-app-server-tests")
                    .join("managed_config.toml")
            } else {
                PathBuf::from(value)
            };
            loader_overrides =
                LoaderOverrides::with_managed_config_path_for_tests(managed_config_path);
        }
    }

    loader_overrides
}
