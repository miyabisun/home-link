//! Matter onboarding codes: QR payloads (`MT:` + Base38) and 11-digit manual pairing codes.

/// What identifies one device across its QR payload and its manual pairing code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetupKey {
    pub passcode: u32,
    pub short_discriminator: u8,
}

/// Returns whether `payload` is a well-formed Matter QR setup payload.
#[must_use]
pub fn is_valid(payload: &str) -> bool {
    payload.starts_with("MT:") && keys(payload).is_some()
}

/// Drops spaces and hyphens from a manual pairing code and returns its 11 digits when valid.
#[must_use]
pub fn normalize_manual(code: &str) -> Option<String> {
    let digits: String = code.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    manual_key(&digits).map(|_| digits)
}

/// The setup keys of a stored code: one per device of a QR payload, one for a manual code.
#[must_use]
pub fn keys(code: &str) -> Option<Vec<SetupKey>> {
    match code.strip_prefix("MT:") {
        Some(rest) => rest.split('*').map(qr_key).collect(),
        None => manual_key(code).map(|key| vec![key]),
    }
}

const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-.";
// The spec rejects these trivially guessable setup passcodes.
const INVALID_PASSCODES: [u32; 12] = [
    0, 11_111_111, 22_222_222, 33_333_333, 44_444_444, 55_555_555, 66_666_666, 77_777_777,
    88_888_888, 99_999_999, 12_345_678, 87_654_321,
];

fn setup_key(passcode: u32, short_discriminator: u32) -> Option<SetupKey> {
    let valid = passcode <= 99_999_998 && !INVALID_PASSCODES.contains(&passcode);
    valid.then_some(SetupKey {
        passcode,
        short_discriminator: u8::try_from(short_discriminator).ok()?,
    })
}

fn qr_key(segment: &str) -> Option<SetupKey> {
    let bytes = decode(segment)?;
    let mut le = [0u8; 16];
    le[..11].copy_from_slice(bytes.get(..11)?);
    let bits = u128::from_le_bytes(le);
    if bits & 0b111 != 0 {
        return None;
    }
    let field = |shift: u32, mask: u128| u32::try_from(bits >> shift & mask).ok();
    // The manual code carries only the top 4 of the 12 discriminator bits.
    setup_key(field(57, 0x7FF_FFFF)?, field(45, 0xFFF)? >> 8)
}

/// Layout: 1 digit (VID/PID flag, discriminator bits 3-2), 5 digits (discriminator
/// bits 1-0, passcode bits 13-0), 4 digits (passcode bits 26-14), Verhoeff check digit.
fn manual_key(code: &str) -> Option<SetupKey> {
    if code.len() != 11 || !code.bytes().all(|b| b.is_ascii_digit()) || !verhoeff_ok(code) {
        return None;
    }
    let number = |range: std::ops::Range<usize>| code[range].parse::<u32>().ok();
    let (first, middle, last) = (number(0..1)?, number(1..6)?, number(6..10)?);
    // A first digit above 3 announces a VID/PID, which only the 21-digit form carries.
    if first > 3 || middle > 0xFFFF || last > 0x1FFF {
        return None;
    }
    setup_key(last << 14 | middle & 0x3FFF, first << 2 | middle >> 14)
}

const VERHOEFF_D: [[u8; 10]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 2, 3, 4, 0, 6, 7, 8, 9, 5],
    [2, 3, 4, 0, 1, 7, 8, 9, 5, 6],
    [3, 4, 0, 1, 2, 8, 9, 5, 6, 7],
    [4, 0, 1, 2, 3, 9, 5, 6, 7, 8],
    [5, 9, 8, 7, 6, 0, 4, 3, 2, 1],
    [6, 5, 9, 8, 7, 1, 0, 4, 3, 2],
    [7, 6, 5, 9, 8, 2, 1, 0, 4, 3],
    [8, 7, 6, 5, 9, 3, 2, 1, 0, 4],
    [9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
];
const VERHOEFF_P: [[u8; 10]; 8] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    [1, 5, 7, 6, 2, 8, 3, 0, 9, 4],
    [5, 8, 0, 3, 7, 9, 6, 1, 4, 2],
    [8, 9, 1, 6, 0, 4, 3, 5, 2, 7],
    [9, 4, 5, 3, 1, 2, 6, 8, 7, 0],
    [4, 2, 8, 6, 5, 7, 3, 9, 0, 1],
    [2, 7, 9, 3, 8, 0, 6, 4, 1, 5],
    [7, 0, 4, 6, 9, 1, 3, 2, 5, 8],
];

/// Whether an all-digit string ends in its correct Verhoeff check digit.
fn verhoeff_ok(digits: &str) -> bool {
    let check = digits.bytes().rev().enumerate().fold(0u8, |c, (i, b)| {
        VERHOEFF_D[usize::from(c)][usize::from(VERHOEFF_P[i % 8][usize::from(b - b'0')])]
    });
    check == 0
}

/// Base38 decoding: 5 chars -> 3 bytes, a trailing 4 -> 2 or 2 -> 1, little-endian.
fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(5) {
        let len = match chunk.len() {
            5 => 3,
            4 => 2,
            2 => 1,
            _ => return None,
        };
        let mut value = 0u32;
        for &c in chunk.iter().rev() {
            let digit = ALPHABET.iter().position(|&a| a == c)?;
            value = value * 38 + u32::try_from(digit).ok()?;
        }
        if value >> (8 * len) != 0 {
            return None;
        }
        out.extend_from_slice(&value.to_le_bytes()[..len]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{ALPHABET, SetupKey, is_valid, keys, normalize_manual, verhoeff_ok};

    // Matter specification example: vendor 0xFFF1, product 0x8000,
    // discriminator 3840, passcode 20202021.
    const SPEC_EXAMPLE: &str = "MT:Y.K9042C00KA0648G00";

    #[test]
    fn accepts_the_specification_example() {
        assert!(is_valid(SPEC_EXAMPLE));
    }

    #[test]
    fn accepts_concatenated_payloads() {
        assert!(is_valid("MT:Y.K9042C00KA0648G00*Y.K9042C00KA0648G00"));
    }

    #[test]
    fn rejects_malformed_payloads() {
        for payload in [
            "",
            "MT:",
            "Y.K9042C00KA0648G00",
            "mt:Y.K9042C00KA0648G00",
            "MT:Y.K9042C00KA0648G0", // 18 chars: too short for the fixed fields
            "MT:y.K9042C00KA0648G00", // lowercase is outside Base38
            "MT:Y.K9042C00KA0648G00 ", // trailing space
            "MT:Y.K9042C00KA0648G00*",
            "MT:Y.K9042C00KA0648G0000", // dangling chunk of length 1
            "MT:.....42C00KA0648G00",   // chunk value exceeds 3 bytes
            "https://example.com/",
        ] {
            assert!(!is_valid(payload), "accepted {payload:?}");
        }
    }

    #[test]
    fn rejects_payloads_with_an_unknown_version_or_invalid_passcode() {
        assert!(!is_valid(&encode(1, 20_202_021)));
        for passcode in [
            0,
            11_111_111,
            12_345_678,
            87_654_321,
            99_999_999,
            100_000_000,
        ] {
            assert!(
                !is_valid(&encode(0, passcode)),
                "accepted passcode {passcode}"
            );
        }
        assert!(is_valid(&encode(0, 20_202_021)));
        assert_eq!(encode(0, 20_202_021), SPEC_EXAMPLE);
    }

    #[test]
    fn manual_codes_normalize_separators_and_check_the_verhoeff_digit() {
        // The spec example's manual pairing code.
        assert_eq!(
            normalize_manual(" 3497-011 2332 ").as_deref(),
            Some("34970112332")
        );
        for code in [
            "",
            "3497011233",             // 10 digits
            "349701123320",           // 12 digits
            "34970112331",            // wrong check digit
            "34970112323",            // swapped digits
            "3497O112332",            // letter O
            "３４９７０１１２３３２", // full-width digits
        ] {
            assert_eq!(normalize_manual(code), None, "accepted {code:?}");
        }
    }

    /// Codes that commissioned real devices; their sharing windows have expired.
    const EXPIRED_SHARED_CODES: [&str; 3] = ["10482735974", "04691737344", "00726849676"];

    #[test]
    fn manual_codes_accept_real_codes_and_reject_any_single_digit_change() {
        for code in EXPIRED_SHARED_CODES {
            assert_eq!(normalize_manual(code).as_deref(), Some(code));
            for i in 0..code.len() {
                for d in b'0'..=b'9' {
                    let mut changed = code.as_bytes().to_vec();
                    if changed[i] == d {
                        continue;
                    }
                    changed[i] = d;
                    let changed = String::from_utf8(changed).unwrap();
                    assert!(!verhoeff_ok(&changed), "accepted {changed}");
                }
            }
        }
        // The fields matter.js decodes from the first code.
        let key = SetupKey {
            passcode: 58_938_075,
            short_discriminator: 4,
        };
        assert_eq!(keys(EXPIRED_SHARED_CODES[0]), Some(vec![key]));
    }

    #[test]
    fn manual_codes_reject_invalid_passcodes_and_out_of_range_chunks() {
        for (short, passcode) in [
            (15, 0),
            (15, 11_111_111),
            (15, 12_345_678),
            (15, 99_999_999),
        ] {
            let code = encode_manual(short, passcode);
            assert_eq!(normalize_manual(&code), None, "accepted {code}");
        }
        assert_eq!(normalize_manual(&with_check("0655360000")), None);
        assert_eq!(normalize_manual(&with_check("0000008192")), None);
        // A VID/PID flag needs the 21-digit form.
        assert_eq!(normalize_manual(&with_check("7497011233")), None);
        assert!(normalize_manual(&encode_manual(0, 1)).is_some());
    }

    #[test]
    fn qr_and_manual_codes_of_one_device_share_a_setup_key() {
        let key = SetupKey {
            passcode: 20_202_021,
            short_discriminator: 15,
        };
        assert_eq!(keys(SPEC_EXAMPLE), Some(vec![key]));
        assert_eq!(keys("34970112332"), Some(vec![key]));
        assert_eq!(
            keys("MT:Y.K9042C00KA0648G00*Y.K9042C00KA0648G00"),
            Some(vec![key, key])
        );
        assert_eq!(
            keys(&encode_manual(14, 20_202_021)).unwrap()[0].short_discriminator,
            14
        );
        assert_ne!(keys(&encode(0, 20_202_022)), Some(vec![key]));
        assert_eq!(keys("MT:abc"), None);
        assert_eq!(keys("34970112331"), None);
    }

    /// Builds an 11-digit manual code from its fields.
    fn encode_manual(short: u32, passcode: u32) -> String {
        let chunk2 = (short & 3) << 14 | passcode & 0x3FFF;
        with_check(&format!("{}{chunk2:05}{:04}", short >> 2, passcode >> 14))
    }

    /// Appends the Verhoeff check digit, searching for the one that validates.
    fn with_check(digits: &str) -> String {
        (0..10)
            .map(|d| format!("{digits}{d}"))
            .find(|code| verhoeff_ok(code))
            .unwrap()
    }

    /// Encodes the spec example's fields with a chosen version and passcode.
    fn encode(version: u128, passcode: u128) -> String {
        let bits = version | 0xFFF1 << 3 | 0x8000 << 19 | 0b10 << 37 | 3840 << 45 | passcode << 57;
        let bytes = &bits.to_le_bytes()[..11];
        let mut out = String::from("MT:");
        for chunk in bytes.chunks(3) {
            let mut value = chunk
                .iter()
                .rev()
                .fold(0u32, |acc, &b| acc << 8 | u32::from(b));
            for _ in 0..[0, 2, 4, 5][chunk.len()] {
                out.push(ALPHABET[(value % 38) as usize] as char);
                value /= 38;
            }
        }
        out
    }
}
