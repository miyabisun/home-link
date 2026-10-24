//! The home's place, from an address through GSI's address search, and the temperature
//! outside it from Open-Meteo; neither needs a key.

use serde::{Deserialize, Serialize};

pub const GSI_URL: &str = "https://msearch.gsi.go.jp/address-search/AddressSearch";
pub const OPEN_METEO_URL: &str = "https://api.open-meteo.com/v1/forecast";
/// The latest stable Chrome on Windows, as the requests go out to public services.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36";
/// How long an outside temperature is used after the time it is for.
pub const FRESH_FOR: i64 = 2 * 3600;

/// Where the home is; the address typed is kept in the database only, never logged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Location {
    pub address: Option<String>,
    /// The address search's name for the place found, `None` until an address is registered.
    pub name: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
}

impl Default for Location {
    /// Tokyo (Shinjuku), where sunrise and sunset were computed before a place was registered.
    fn default() -> Self {
        Self {
            address: None,
            name: None,
            latitude: 35.6895,
            longitude: 139.6917,
        }
    }
}

/// The first place GSI's address search answers with: its name, latitude and longitude.
#[must_use]
pub fn place(body: &str) -> Option<(String, f64, f64)> {
    let found: serde_json::Value = serde_json::from_str(body).ok()?;
    let first = found.get(0)?;
    let name = first["properties"]["title"]
        .as_str()
        .filter(|t| !t.is_empty())?;
    // GeoJSON's order: longitude, then latitude.
    let coordinates = first["geometry"]["coordinates"].as_array()?;
    let (longitude, latitude) = (
        coordinates.first()?.as_f64()?,
        coordinates.get(1)?.as_f64()?,
    );
    ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude))
        .then(|| (name.to_owned(), latitude, longitude))
}

/// The temperature outside and the Unix second it is for, from Open-Meteo's `current`.
#[must_use]
pub fn temperature(body: &str) -> Option<(f64, i64)> {
    let answer: serde_json::Value = serde_json::from_str(body).ok()?;
    let current = &answer["current"];
    let celsius = current["temperature_2m"].as_f64()?;
    // `2026-10-24T07:15` in the answer's time zone, GMT unless asked otherwise.
    let time = current["time"].as_str()?;
    let number = |range: std::ops::Range<usize>| time.get(range)?.parse::<i64>().ok();
    let shape = time.len() == 16 && &time[4..5] == "-" && &time[10..11] == "T";
    if !shape {
        return None;
    }
    let days = crate::schedule::days_from_civil(number(0..4)?, number(5..7)?, number(8..10)?);
    let local = days * 86_400 + number(11..13)? * 3600 + number(14..16)? * 60;
    Some((
        celsius,
        local - answer["utc_offset_seconds"].as_i64().unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GSI's answer for 東京都新宿区西新宿2-8-1 on 2026-10-24.
    const GSI: &str = r#"[{"geometry":{"coordinates":[139.691772,35.689629],"type":"Point"},"type":"Feature","properties":{"addressCode":"","title":"東京都新宿区西新宿二丁目８番"}}]"#;
    /// Open-Meteo's answer for Shinjuku on 2026-10-24.
    const OPEN_METEO: &str = r#"{"latitude":35.7,"longitude":139.6875,"generationtime_ms":0.024080276489257812,"utc_offset_seconds":0,"timezone":"GMT","timezone_abbreviation":"GMT","elevation":40.0,"current_units":{"time":"iso8601","interval":"seconds","temperature_2m":"°C"},"current":{"time":"2026-10-24T07:15","interval":900,"temperature_2m":15.2}}"#;

    #[test]
    fn reads_the_first_place_found() {
        assert_eq!(
            place(GSI),
            Some((
                "東京都新宿区西新宿二丁目８番".into(),
                35.689_629,
                139.691_772
            ))
        );
        let two = format!(
            "[{},{}]",
            &GSI[1..GSI.len() - 1],
            GSI[1..GSI.len() - 1].replace("西新宿二丁目８番", "どこか")
        );
        assert_eq!(place(&two).unwrap().0, "東京都新宿区西新宿二丁目８番");
    }

    #[test]
    fn finds_no_place_in_an_empty_or_odd_answer() {
        for body in [
            "[]",
            "",
            "<html>",
            r#"[{"geometry":{"coordinates":[139.6]},"properties":{"title":"x"}}]"#,
            r#"[{"geometry":{"coordinates":[200.0,35.6]},"properties":{"title":"x"}}]"#,
            r#"[{"geometry":{"coordinates":[139.6,35.6]},"properties":{"title":""}}]"#,
        ] {
            assert_eq!(place(body), None, "{body}");
        }
    }

    #[test]
    fn reads_the_current_temperature_and_its_time() {
        // 2026-10-24T07:15 GMT.
        assert_eq!(temperature(OPEN_METEO), Some((15.2, 1_792_826_100)));
        let tokyo = OPEN_METEO
            .replace(r#""utc_offset_seconds":0"#, r#""utc_offset_seconds":32400"#)
            .replace("07:15", "16:15");
        assert_eq!(temperature(&tokyo), Some((15.2, 1_792_826_100)));
    }

    #[test]
    fn reads_no_temperature_from_an_odd_answer() {
        for body in [
            "",
            "{}",
            r#"{"current":{"time":"2026-10-24T07:15"}}"#,
            r#"{"current":{"time":"yesterday","temperature_2m":15.2}}"#,
            r#"{"current":{"temperature_2m":15.2}}"#,
        ] {
            assert_eq!(temperature(body), None, "{body}");
        }
    }
}
