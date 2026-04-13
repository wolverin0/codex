use anyhow::Result;
use codex_config::CONFIG_TOML_FILE;
use codex_core::plugins::marketplace_install_root;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

fn write_marketplace_source(source: &Path, marker: &str) -> Result<()> {
    std::fs::create_dir_all(source.join(".agents/plugins"))?;
    std::fs::create_dir_all(source.join("plugins/sample/.codex-plugin"))?;
    std::fs::write(
        source.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "debug",
  "plugins": [
    {
      "name": "sample",
      "source": {
        "source": "local",
        "path": "./plugins/sample"
      }
    }
  ]
}"#,
    )?;
    std::fs::write(
        source.join("plugins/sample/.codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    )?;
    std::fs::write(source.join("plugins/sample/marker.txt"), marker)?;
    Ok(())
}

fn read_user_config(codex_home: &Path) -> Result<String> {
    Ok(std::fs::read_to_string(codex_home.join(CONFIG_TOML_FILE))?)
}

fn marketplace_config(config: &str, marketplace_name: &str) -> Result<toml::Value> {
    let config: toml::Value = toml::from_str(config)?;
    Ok(config["marketplaces"][marketplace_name].clone())
}

fn free_local_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn wait_for_http_server(port: u16) {
    for _ in 0..20 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("python http server did not start on port {port}");
}

#[tokio::test]
async fn marketplace_add_supports_local_directory_source() -> Result<()> {
    let codex_home = TempDir::new()?;
    let source = TempDir::new()?;
    write_marketplace_source(source.path(), "local ref")?;
    let source_parent = source.path().parent().unwrap();
    let source_arg = format!("./{}", source.path().file_name().unwrap().to_string_lossy());

    codex_command(codex_home.path())?
        .current_dir(source_parent)
        .args(["marketplace", "add", source_arg.as_str()])
        .assert()
        .success();

    let installed_root = marketplace_install_root(codex_home.path()).join("debug");
    assert_eq!(
        std::fs::read_to_string(installed_root.join("plugins/sample/marker.txt"))?,
        "local ref"
    );

    let config = read_user_config(codex_home.path())?;
    let marketplace = marketplace_config(&config, "debug")?;
    let expected_source = source.path().canonicalize()?.display().to_string();
    assert_eq!(marketplace["source_type"].as_str(), Some("path"));
    assert_eq!(
        marketplace["source"].as_str(),
        Some(expected_source.as_str())
    );

    Ok(())
}

#[tokio::test]
async fn marketplace_add_supports_manifest_url_source() -> Result<()> {
    let codex_home = TempDir::new()?;
    let source = TempDir::new()?;
    std::fs::create_dir_all(source.path().join(".agents/plugins"))?;
    std::fs::write(
        source.path().join(".agents/plugins/marketplace.json"),
        r#"{"name":"debug-url","plugins":[]}"#,
    )?;
    let port = free_local_port()?;
    let mut server = Command::new("python3")
        .args([
            "-m",
            "http.server",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
            "--directory",
            source.path().to_str().expect("utf-8 tempdir path"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    wait_for_http_server(port);
    let url = format!("http://127.0.0.1:{port}/.agents/plugins/marketplace.json");

    codex_command(codex_home.path())?
        .args(["marketplace", "add", &url])
        .assert()
        .success();

    let installed_root = marketplace_install_root(codex_home.path()).join("debug-url");
    assert!(
        installed_root
            .join(".agents/plugins/marketplace.json")
            .is_file()
    );

    let config = read_user_config(codex_home.path())?;
    let marketplace = marketplace_config(&config, "debug-url")?;
    assert_eq!(marketplace["source_type"].as_str(), Some("manifest_url"));
    assert_eq!(marketplace["source"].as_str(), Some(url.as_str()));
    let _ = server.kill();
    let _ = server.wait();

    Ok(())
}
