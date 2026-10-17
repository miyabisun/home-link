//! Unlocking and locking the front door's SESAME through the CANDY HOUSE Web API and its Wi-Fi module.
//! The request follows <https://github.com/CANDY-HOUSE/API_document> (`SesameOS3/webapi.md`).

use std::{fmt, time::Duration};

use aes::Aes128;
use cmac::{Cmac, Mac};
use reqwest::header::HeaderValue;

pub const API_URL: &str = "https://app.candyhouse.co";
/// `home-link` in base64, the name the SESAME app shows in its history (21 bytes at most).
const HISTORY: &str = "aG9tZS1saW5r";
const TIMEOUT: Duration = Duration::from_secs(10);

const NOT_CONFIGURED: &str =
    "SESAMEの設定（SESAME_API_KEY・SESAME_UUID・SESAME_SECRET_KEY）がありません";
const INVALID_API_KEY: &str = "SESAME_API_KEYは英数字と記号で設定してください";
const INVALID_UUID: &str = "SESAME_UUIDは英数字とハイフンで設定してください";
const INVALID_SECRET: &str = "SESAME_SECRET_KEYは32桁の16進数で設定してください";

/// Where and how to reach one SESAME. `Debug` leaves the keys and the UUID out.
#[derive(Clone)]
pub struct Config {
    api_key: HeaderValue,
    uuid: String,
    secret: [u8; 16],
    api_url: String,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("api_url", &self.api_url)
            .finish_non_exhaustive()
    }
}

impl Config {
    /// Builds the configuration from the three settings; any of them missing or blank means unset.
    ///
    /// # Errors
    /// A message naming the setting that is missing or malformed, without its value.
    pub fn new(
        api_key: Option<&str>,
        uuid: Option<&str>,
        secret_key: Option<&str>,
        api_url: &str,
    ) -> Result<Self, &'static str> {
        let (Some(api_key), Some(uuid), Some(secret_key)) =
            (present(api_key), present(uuid), present(secret_key))
        else {
            return Err(NOT_CONFIGURED);
        };
        // The UUID becomes part of the request path.
        if !uuid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(INVALID_UUID);
        }
        let mut api_key = HeaderValue::from_str(api_key).map_err(|_| INVALID_API_KEY)?;
        api_key.set_sensitive(true);
        Ok(Self {
            api_key,
            uuid: uuid.to_owned(),
            secret: parse_secret(secret_key).ok_or(INVALID_SECRET)?,
            api_url: api_url.trim_end_matches('/').to_owned(),
        })
    }

    /// Reads `SESAME_API_KEY`, `SESAME_UUID` and `SESAME_SECRET_KEY`.
    ///
    /// # Errors
    /// As [`Config::new`].
    pub fn from_env() -> Result<Self, &'static str> {
        let var = |name| std::env::var(name).ok();
        Self::new(
            var("SESAME_API_KEY").as_deref(),
            var("SESAME_UUID").as_deref(),
            var("SESAME_SECRET_KEY").as_deref(),
            API_URL,
        )
    }
}

fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// What to have the SESAME do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Lock,
    Unlock,
}

impl Command {
    /// The CANDY HOUSE command code (toggle 88, lock 82, unlock 83).
    const fn code(self) -> u8 {
        match self {
            Self::Lock => 82,
            Self::Unlock => 83,
        }
    }
}

/// Why a command was not accepted.
#[derive(Debug, PartialEq)]
pub enum Failure {
    /// CANDY HOUSE answered with this HTTP status.
    Rejected(u16),
    /// No answer: the connection failed, dropped or timed out.
    Unreachable(String),
}

/// Sends `command` signed for `unix` seconds; `Ok` once CANDY HOUSE accepts it.
///
/// # Errors
/// [`Failure`] when CANDY HOUSE refuses the command or cannot be reached.
pub async fn send(config: &Config, command: Command, unix: i64) -> Result<(), Failure> {
    let body = serde_json::json!({
        "cmd": command.code(),
        "history": HISTORY,
        "sign": sign(&config.secret, unix),
    });
    let response = reqwest::Client::new()
        .post(format!(
            "{}/api/sesame2/{}/cmd",
            config.api_url, config.uuid
        ))
        .header("x-api-key", config.api_key.clone())
        .header("content-type", "application/json")
        .body(body.to_string())
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|error| Failure::Unreachable(error.without_url().to_string()))?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(Failure::Rejected(status.as_u16()))
    }
}

/// The signature CANDY HOUSE checks: AES-CMAC with the SESAME's secret key over
/// bytes 2–4 of the Unix seconds in little endian, as lowercase hex.
fn sign(secret: &[u8; 16], unix: i64) -> String {
    cmac_hex(secret, &signed_bytes(unix))
}

/// Bytes 2–4 of `unix` as a 4-byte little-endian number.
fn signed_bytes(unix: i64) -> [u8; 3] {
    // The low 32 bits, as Python's `to_bytes(4, 'little')` would hold them until 2106.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let le = (unix as u32).to_le_bytes();
    [le[1], le[2], le[3]]
}

fn cmac_hex(key: &[u8; 16], message: &[u8]) -> String {
    use std::fmt::Write;
    let mut mac = <Cmac<Aes128> as Mac>::new(key.into());
    mac.update(message);
    mac.finalize()
        .into_bytes()
        .iter()
        .fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

fn parse_secret(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut secret = [0; 16];
    for (i, byte) in secret.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &str) -> Vec<u8> {
        (0..bytes.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&bytes[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn cmac_matches_the_rfc_4493_examples() {
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let message = hex(concat!(
            "6bc1bee22e409f96e93d7e117393172a",
            "ae2d8a571e03ac9c9eb76fac45af8e51",
            "30c81c46a35ce411e5fbc1191a0a52ef",
            "f69f2445df4f9b17ad2b417be66c3710",
        ));
        for (len, tag) in [
            (0, "bb1d6929e95937287fa37d129b756746"),
            (16, "070a16b46b4d4144f79bdd9dd04a287c"),
            (40, "dfa66747de9ae63030ca32611497c827"),
            (64, "51f0bebf7e3b9d92fc49741779363cfe"),
        ] {
            assert_eq!(cmac_hex(&key, &message[..len]), tag, "{len} bytes");
        }
    }

    #[test]
    fn the_signature_covers_bytes_two_to_four_of_the_little_endian_seconds() {
        assert_eq!(signed_bytes(0x1234_5678), [0x56, 0x34, 0x12]);
        assert_eq!(signed_bytes(1_791_856_659), {
            let le = 1_791_856_659_u32.to_le_bytes();
            [le[1], le[2], le[3]]
        });
        let key = [0x2b; 16];
        assert_eq!(sign(&key, 0x1234_5678), cmac_hex(&key, &[0x56, 0x34, 0x12]));
    }

    #[test]
    fn the_secret_key_is_32_hex_digits() {
        assert_eq!(
            parse_secret("2b7e151628aed2a6abf7158809cf4f3c")
                .unwrap()
                .to_vec(),
            hex("2b7e151628aed2a6abf7158809cf4f3c")
        );
        assert_eq!(
            parse_secret("2B7E151628AED2A6ABF7158809CF4F3C")
                .unwrap()
                .to_vec(),
            hex("2b7e151628aed2a6abf7158809cf4f3c")
        );
        for bad in [
            "",
            "2b7e151628aed2a6abf7158809cf4f3",
            "2b7e151628aed2a6abf7158809cf4f3c00",
            "zz7e151628aed2a6abf7158809cf4f3c",
            "+b7e151628aed2a6abf7158809cf4f3c",
            "２b7e151628aed2a6abf7158809cf4f3",
        ] {
            assert_eq!(parse_secret(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn configuration_needs_all_three_settings_and_well_formed_values() {
        let secret = "2b7e151628aed2a6abf7158809cf4f3c";
        let uuid = "3DE4DE72-AAF9-25C1-8D0F-C9E019BB060C";
        let config = Config::new(Some("key"), Some(uuid), Some(secret), "http://x/").unwrap();
        assert_eq!(config.api_url, "http://x");
        for (key, id, sk) in [
            (None, Some(uuid), Some(secret)),
            (Some("key"), None, Some(secret)),
            (Some("key"), Some(uuid), None),
            (Some(" "), Some(uuid), Some(secret)),
        ] {
            assert_eq!(
                Config::new(key, id, sk, API_URL).unwrap_err(),
                NOT_CONFIGURED
            );
        }
        for id in ["../x", "a/b", "a?b", "a b"] {
            assert_eq!(
                Config::new(Some("key"), Some(id), Some(secret), API_URL).unwrap_err(),
                INVALID_UUID
            );
        }
        assert_eq!(
            Config::new(Some("key"), Some(uuid), Some("abc"), API_URL).unwrap_err(),
            INVALID_SECRET
        );
        assert_eq!(
            Config::new(Some("k\ney"), Some(uuid), Some(secret), API_URL).unwrap_err(),
            INVALID_API_KEY
        );
    }

    #[test]
    fn debug_leaves_the_keys_out() {
        let shown = format!(
            "{:?}",
            Config::new(
                Some("api-key-value"),
                Some("3DE4DE72-AAF9-25C1-8D0F-C9E019BB060C"),
                Some("2b7e151628aed2a6abf7158809cf4f3c"),
                API_URL
            )
            .unwrap()
        );
        assert!(shown.contains(API_URL), "{shown}");
        assert!(!shown.contains("api-key-value"), "{shown}");
        assert!(!shown.to_lowercase().contains("3de4de72"), "{shown}");
        assert!(!shown.contains("2b7e") && !shown.contains("43"), "{shown}");
    }
}
