use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn mfa_js_dir() -> PathBuf {
    if let Ok(d) = std::env::var("MFA_JS_DIR") {
        return PathBuf::from(d);
    }
    if Path::new("mfa-cli.mjs").exists() {
        return PathBuf::from(".");
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(rel) = exe.parent() {
            let benten = rel.join("../../..");
            if benten.join("mfa-cli.mjs").exists() {
                return benten;
            }
            if rel.join("mfa-cli.mjs").exists() {
                return rel.to_path_buf();
            }
        }
    }
    PathBuf::from(".")
}

#[derive(Clone, Debug)]
pub struct MfaResult {
    pub token: String,
    pub cookie: String,
}

async fn wait_abort(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn parse_mfa_output(output: std::process::Output) -> Result<MfaResult> {
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Node MFA fail: {}", err.trim()));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Err(anyhow!("Node MFA bos stdout"));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
        if let Some(t) = v.get("token").and_then(|x| x.as_str()) {
            if t.len() < 20 {
                return Err(anyhow!("Node MFA bos token"));
            }
            return Ok(MfaResult {
                token: t.to_string(),
                cookie: v
                    .get("cookie")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    if raw.len() < 20 {
        return Err(anyhow!("Node MFA bos token"));
    }
    Ok(MfaResult {
        token: raw,
        cookie: String::new(),
    })
}

async fn run_mfa_node(
    token: &str,
    password: &str,
    gid: &str,
    cancel: Option<&AtomicBool>,
) -> Result<MfaResult> {
    let password = std::env::var("MFA_PASSWORD").unwrap_or_else(|_| password.to_string());
    let dir = mfa_js_dir();
    let script = dir.join("mfa-cli.mjs");
    if !script.exists() {
        return Err(anyhow!(
            "mfa-cli.mjs bulunamadi ({}). kod klasorunden calistir.",
            script.display()
        ));
    }

    tracing::info!("[mfa] Node rush-mfa gid={gid}…");

    let child = tokio::process::Command::new("node")
        .arg(&script)
        .env("MFA_TOKEN", token)
        .env("MFA_PASSWORD", password)
        .env("MFA_GID", gid)
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("node mfa-cli spawn")?;

    let output = if let Some(cancel) = cancel {
        tokio::select! {
            out = child.wait_with_output() => out.context("node mfa wait")?,
            () = wait_abort(cancel) => {
                return Err(anyhow!("MFA aborted"));
            }
        }
    } else {
        child.wait_with_output().await.context("node mfa wait")?
    };

    parse_mfa_output(output)
}

/// MFA — Node.js rush-mfa (discord.com TLS) | fire Rust'ta kalır
pub async fn fetch_mfa(token: &str, password: &str, gid: &str) -> Result<MfaResult> {
    run_mfa_node(token, password, gid, None).await
}

/// Boost/fire sırasında iptal edilebilir MFA
pub async fn fetch_mfa_cancellable(
    token: &str,
    password: &str,
    gid: &str,
    cancel: &AtomicBool,
) -> Result<MfaResult> {
    run_mfa_node(token, password, gid, Some(cancel)).await
}
