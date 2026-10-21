use anyhow::Result;
use reqwest::Client;
use serde_json::json;

pub async fn send_webhook(url: &str, title: &str, description: &str, color: u32) -> Result<()> {
    send_webhook_with_content(url, title, description, color, None).await
}

pub async fn send_webhook_with_content(
    url: &str,
    title: &str,
    description: &str,
    color: u32,
    content: Option<&str>,
) -> Result<()> {
    if !url.contains("webhooks") {
        return Ok(());
    }
    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .use_rustls_tls()
        .build()?;
    let mut body = json!({
        "embeds": [{
            "title": title,
            "description": &description[..description.len().min(3900)],
            "color": color
        }]
    });
    if let Some(c) = content.filter(|s| !s.is_empty()) {
        body["content"] = json!(c);
    }
    let _ = client.post(url).json(&body).send().await?;
    Ok(())
}
