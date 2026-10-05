use std::env;

const INVALID_PORT: &str = "PORT must be a decimal TCP port from 1 to 65535";

pub fn from_env() -> Result<u16, &'static str> {
    match env::var("PORT") {
        Ok(value) => parse(Some(&value)),
        Err(env::VarError::NotPresent) => parse(None),
        Err(env::VarError::NotUnicode(_)) => Err(INVALID_PORT),
    }
}

fn parse(value: Option<&str>) -> Result<u16, &'static str> {
    let Some(value) = value else {
        return Ok(3000);
    };
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(INVALID_PORT);
    }
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or(INVALID_PORT)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn absent_port_defaults_to_3000() {
        assert_eq!(parse(None), Ok(3000));
    }

    #[test]
    fn accepts_decimal_ports_in_range() {
        for (value, port) in [("1", 1), ("3100", 3100), ("65535", 65535), ("03000", 3000)] {
            assert_eq!(parse(Some(value)), Ok(port));
        }
    }

    #[test]
    fn rejects_invalid_and_out_of_range_ports() {
        for value in [
            "",
            "0",
            "65536",
            "abc",
            "-1",
            "+3000",
            " 3000",
            "3000 ",
            "127.0.0.1:3000",
        ] {
            assert!(parse(Some(value)).is_err(), "accepted {value:?}");
        }
    }
}
