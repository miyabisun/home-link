//! Messages to the user through a Discord webhook.

use std::time::Duration;

/// The variable that holds the webhook: its URL, or Watchtower's (shoutrrr's) `discord://token@id`.
pub const ENV: &str = "DISCORD_WEBHOOK_URL";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Discord refuses a message longer than this many characters.
const MAX_CHARS: usize = 2000;

/// Where messages go. Its URL holds the webhook's token and is never logged.
#[derive(Clone)]
pub struct Discord {
    url: String,
}

impl std::fmt::Debug for Discord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Discord(..)")
    }
}

impl Discord {
    /// The webhook `value` names, `None` when it names none.
    #[must_use]
    pub fn new(value: &str) -> Option<Self> {
        webhook(value).map(|url| Self { url })
    }

    /// Posts `text`, cut to what Discord takes.
    ///
    /// # Errors
    /// The reason it was not delivered, without the URL.
    pub async fn send(&self, text: &str) -> Result<(), String> {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::json!({ "content": cut(text) }).to_string())
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map(|_| ())
            .map_err(|e| e.without_url().to_string())
    }
}

/// The webhook URL of `value`: an `http(s)` URL as it is, or shoutrrr's `discord://token@id`.
fn webhook(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with("https://") || value.starts_with("http://") {
        return Some(value.to_owned());
    }
    let rest = value.strip_prefix("discord://")?;
    let rest = rest.split(['?', '/']).next()?;
    let (token, id) = rest.split_once('@')?;
    (!token.is_empty() && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("https://discord.com/api/webhooks/{id}/{token}"))
}

fn cut(text: &str) -> String {
    if text.chars().count() <= MAX_CHARS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(MAX_CHARS - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::{MAX_CHARS, cut, webhook};

    #[test]
    fn a_webhook_is_its_url_or_watchtowers_discord_url() {
        assert_eq!(
            webhook("https://discord.com/api/webhooks/123/abc-DEF").as_deref(),
            Some("https://discord.com/api/webhooks/123/abc-DEF")
        );
        assert_eq!(
            webhook("discord://abc-DEF@123").as_deref(),
            Some("https://discord.com/api/webhooks/123/abc-DEF")
        );
        assert_eq!(
            webhook(" discord://abc@123?splitlines=no ").as_deref(),
            Some("https://discord.com/api/webhooks/123/abc")
        );
        assert_eq!(webhook(""), None);
        assert_eq!(webhook("discord://abc"), None);
        assert_eq!(webhook("discord://@123"), None);
        assert_eq!(webhook("discord://abc@channel"), None);
        assert_eq!(webhook("slack://abc@123"), None);
    }

    #[test]
    fn a_long_message_is_cut_to_what_discord_takes() {
        let long = "照".repeat(MAX_CHARS + 5);
        assert_eq!(cut(&long).chars().count(), MAX_CHARS);
        assert!(cut(&long).ends_with('…'));
        assert_eq!(cut("短い"), "短い");
    }
}
