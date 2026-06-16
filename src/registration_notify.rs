use std::sync::OnceLock;
use std::time::Duration;
use tracing::warn;

/// Wall-clock budget for awaiting the webhook before the handler returns.
/// Keeps Vercel invocations reliable without adding large latency.
const NOTIFY_AWAIT_TIMEOUT: Duration = Duration::from_secs(2);
/// Per-request HTTP timeout (must be ≤ [`NOTIFY_AWAIT_TIMEOUT`]).
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);

fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("pr402-registration-notify/1")
            .build()
            .expect("reqwest Client::builder().build()")
    })
}

fn webhook_url() -> Option<String> {
    crate::parameters::resolve_string_sync(
        crate::parameters::PR402_REGISTRATION_NOTIFICATION_WEBHOOK_URL,
        crate::parameters::PR402_REGISTRATION_NOTIFICATION_WEBHOOK_URL,
    )
}

/// Check if resource registration notifications are enabled.
/// Default to true if not configured.
pub fn resource_notify_enabled() -> bool {
    if let Some(val) = crate::parameters::resolve_string_sync(
        crate::parameters::PR402_RESOURCE_REGISTRATION_NOTIFICATION,
        crate::parameters::PR402_RESOURCE_REGISTRATION_NOTIFICATION,
    ) {
        matches!(
            val.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    } else {
        true
    }
}

fn is_discord_webhook(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("discord.com") || lower.contains("discordapp.com")
}

fn bold_label(is_discord: bool, label: &str) -> String {
    if is_discord {
        format!("**{label}**")
    } else {
        format!("*{label}*")
    }
}

/// Human-readable deployment label from Vercel's runtime-injected `VERCEL_ENV`.
fn deployment_label() -> &'static str {
    match std::env::var("VERCEL_ENV")
        .ok()
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(v) if v.eq_ignore_ascii_case("production") => "Production",
        Some(v) if v.eq_ignore_ascii_case("preview") => "Preview",
        Some(v) if v.eq_ignore_ascii_case("development") => "Development",
        _ => "Local",
    }
}

fn notification_header_tag() -> String {
    format!("(pr402 · {})", deployment_label())
}

fn seller_notification_text(is_discord: bool, wallet: &str, service_url: Option<&str>) -> String {
    format!(
        "📢 {} {}\n• {} `{}`\n• {} {}",
        bold_label(is_discord, "New Seller Onboarded"),
        notification_header_tag(),
        bold_label(is_discord, "Wallet:"),
        wallet,
        bold_label(is_discord, "Service URL:"),
        service_url.unwrap_or("N/A")
    )
}

fn resource_notification_text(is_discord: bool, wallet: &str, resource_url: &str) -> String {
    format!(
        "🚀 {} {}\n• {} `{}`\n• {} {}",
        bold_label(is_discord, "New Resource Registered"),
        notification_header_tag(),
        bold_label(is_discord, "Wallet:"),
        wallet,
        bold_label(is_discord, "Resource URL:"),
        resource_url
    )
}

/// Await a registration webhook (up to [`NOTIFY_AWAIT_TIMEOUT`]) after seller DB upsert succeeds.
pub async fn spawn_seller_notify(wallet: &str, service_url: Option<&str>) {
    let Some(url) = webhook_url() else {
        return;
    };
    let is_discord = is_discord_webhook(&url);
    let text = seller_notification_text(is_discord, wallet, service_url);
    await_registration_webhook("seller", &url, text).await;
}

/// Await a registration webhook (up to [`NOTIFY_AWAIT_TIMEOUT`]) after resource DB upsert succeeds.
pub async fn spawn_resource_notify(wallet: &str, resource_url: &str) {
    if !resource_notify_enabled() {
        return;
    }
    let Some(url) = webhook_url() else {
        return;
    };
    let is_discord = is_discord_webhook(&url);
    let text = resource_notification_text(is_discord, wallet, resource_url);
    await_registration_webhook("resource", &url, text).await;
}

async fn await_registration_webhook(kind: &str, url: &str, text: String) {
    match tokio::time::timeout(NOTIFY_AWAIT_TIMEOUT, send_notification_webhook(url, text)).await {
        Ok(()) => {}
        Err(_) => {
            warn!(
                target: "server_log",
                kind = kind,
                timeout_secs = NOTIFY_AWAIT_TIMEOUT.as_secs(),
                "Registration notification webhook timed out before handler returned"
            );
        }
    }
}

/// Sends a JSON payload to Slack or Discord webhook.
async fn send_notification_webhook(url: &str, text: String) {
    let is_discord = is_discord_webhook(url);
    let payload = if is_discord {
        serde_json::json!({ "content": text })
    } else {
        serde_json::json!({ "text": text })
    };

    match shared_client().post(url).json(&payload).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    target: "server_log",
                    status = %status,
                    body = %body,
                    "Registration notification webhook returned non-success status"
                );
            }
        }
        Err(e) => {
            warn!(
                target: "server_log",
                error = %e,
                "Failed to send registration notification webhook"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV_KEY: &str = crate::parameters::PR402_RESOURCE_REGISTRATION_NOTIFICATION;
    const VERCEL_ENV_KEY: &str = "VERCEL_ENV";

    fn with_env(val: Option<&str>, f: impl FnOnce()) {
        let prior = std::env::var(ENV_KEY).ok();
        match val {
            Some(v) => std::env::set_var(ENV_KEY, v),
            None => std::env::remove_var(ENV_KEY),
        }
        f();
        match prior {
            Some(v) => std::env::set_var(ENV_KEY, v),
            None => std::env::remove_var(ENV_KEY),
        }
    }

    fn with_vercel_env(val: Option<&str>, f: impl FnOnce()) {
        let prior = std::env::var(VERCEL_ENV_KEY).ok();
        match val {
            Some(v) => std::env::set_var(VERCEL_ENV_KEY, v),
            None => std::env::remove_var(VERCEL_ENV_KEY),
        }
        f();
        match prior {
            Some(v) => std::env::set_var(VERCEL_ENV_KEY, v),
            None => std::env::remove_var(VERCEL_ENV_KEY),
        }
    }

    #[test]
    fn resource_notify_enabled_semantics() {
        with_env(None, || assert!(resource_notify_enabled()));

        for val in ["1", "true", "yes", "on", "TRUE", "Yes", "ON"] {
            with_env(Some(val), || {
                assert!(resource_notify_enabled(), "value {val} should be truthy")
            });
        }

        for val in ["0", "false", "no", "off", "random_string"] {
            with_env(Some(val), || {
                assert!(!resource_notify_enabled(), "value {val} should be falsy")
            });
        }
    }

    #[test]
    fn notification_header_and_formatting() {
        with_vercel_env(None, || {
            assert_eq!(deployment_label(), "Local");
            let slack =
                seller_notification_text(false, "wallet123", Some("https://api.example.com"));
            assert!(slack.contains("*New Seller Onboarded*"));
            assert!(slack.contains("*Wallet:*"));
            assert!(slack.contains("(pr402 · Local)"));
            assert!(!slack.contains("**"));

            let discord =
                seller_notification_text(true, "wallet123", Some("https://api.example.com"));
            assert!(discord.contains("**New Seller Onboarded**"));
            assert!(discord.contains("**Wallet:**"));
            assert!(discord.contains("(pr402 · Local)"));
        });

        with_vercel_env(Some("production"), || {
            assert_eq!(deployment_label(), "Production");
            assert!(seller_notification_text(false, "w", None).contains("(pr402 · Production)"));
        });

        with_vercel_env(Some("preview"), || {
            assert_eq!(deployment_label(), "Preview");
            assert!(
                resource_notification_text(false, "w", "https://api.example.com/x")
                    .contains("(pr402 · Preview)")
            );
        });

        with_vercel_env(Some("development"), || {
            assert_eq!(deployment_label(), "Development");
        });
    }

    #[test]
    fn is_discord_webhook_detects_discord_hosts() {
        assert!(is_discord_webhook(
            "https://discord.com/api/webhooks/123/abc"
        ));
        assert!(is_discord_webhook(
            "https://discordapp.com/api/webhooks/123/abc"
        ));
        assert!(!is_discord_webhook(
            "https://hooks.slack.com/services/T/B/xxx"
        ));
    }
}
