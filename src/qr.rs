//! Validation of Matter onboarding QR payloads (`MT:` + Base38).

/// Returns whether `payload` is a well-formed Matter QR setup payload.
#[must_use]
pub fn is_valid(payload: &str) -> bool {
    payload
        .strip_prefix("MT:")
        .is_some_and(|rest| rest.split('*').all(is_valid_segment))
}

const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-.";
// The spec rejects these trivially guessable setup passcodes.
const INVALID_PASSCODES: [u32; 12] = [
    0, 11_111_111, 22_222_222, 33_333_333, 44_444_444, 55_555_555, 66_666_666, 77_777_777,
    88_888_888, 99_999_999, 12_345_678, 87_654_321,
];

fn is_valid_segment(segment: &str) -> bool {
    let Some(bytes) = decode(segment) else {
        return false;
    };
    let Some(fixed) = bytes.get(..11) else {
        return false;
    };
    let mut le = [0u8; 16];
    le[..11].copy_from_slice(fixed);
    let bits = u128::from_le_bytes(le);
    let version = bits & 0b111;
    let passcode = u32::try_from(bits >> 57 & 0x7FF_FFFF).unwrap_or(u32::MAX);
    version == 0 && passcode <= 99_999_998 && !INVALID_PASSCODES.contains(&passcode)
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
    use super::{ALPHABET, is_valid};

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
