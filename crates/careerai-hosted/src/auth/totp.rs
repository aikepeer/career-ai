//! TOTP (RFC 6238) implementation for mandatory MFA.
//!
//! Generates and verifies time-based one-time passwords using HMAC-SHA1
//! with a 30-second step and 6-digit output. Supports a ±1 window for
//! clock drift.

use constant_time_eq::constant_time_eq;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;
use thiserror::Error;

type HmacSha1 = Hmac<Sha1>;

#[derive(Debug, Error)]
pub enum TotpError {
    #[error("TOTP code does not match")]
    Invalid,
    #[error("TOTP secret is not valid base32")]
    InvalidSecret,
}

/// TOTP configuration per RFC 6238.
const STEP_SECONDS: u64 = 30;
const DIGITS: u32 = 6;
/// Allowed window: ±1 step (90-second total acceptance window).
const WINDOW: u64 = 1;

/// TOTP generator/verifier.
#[derive(Debug, Clone)]
pub struct Totp {
    /// Raw secret bytes (decoded from base32).
    secret: Vec<u8>,
}

impl Totp {
    /// Create a new TOTP from a base32-encoded secret.
    pub fn from_base32(secret: &str) -> Result<Self, TotpError> {
        let bytes = decode_base32(secret)
            .ok_or(TotpError::InvalidSecret)?;
        Ok(Self { secret: bytes })
    }

    /// Generate a new random TOTP secret and return (base32_secret, Totp).
    pub fn generate() -> (String, Self) {
        let mut bytes = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut bytes);
        let b32 = encode_base32(&bytes);
        let totp = Self {
            secret: bytes.to_vec(),
        };
        (b32, totp)
    }

    /// Generate the current TOTP code for the given Unix timestamp.
    pub fn code_at(&self, unix_seconds: u64) -> String {
        let counter = unix_seconds / STEP_SECONDS;
        compute_totp(&self.secret, counter, DIGITS)
    }

    /// Generate the current TOTP code.
    pub fn now(&self) -> String {
        self.code_at(current_unix())
    }

    /// Verify a TOTP code with a ±1 step window.
    pub fn verify(&self, code: &str, unix_seconds: u64) -> Result<(), TotpError> {
        let counter = unix_seconds / STEP_SECONDS;
        for offset in 0..=WINDOW {
            let expected = compute_totp(&self.secret, counter - offset, DIGITS);
            if constant_time_eq(expected.as_bytes(), code.as_bytes()) {
                return Ok(());
            }
            if offset > 0 {
                let expected = compute_totp(&self.secret, counter + offset, DIGITS);
                if constant_time_eq(expected.as_bytes(), code.as_bytes()) {
                    return Ok(());
                }
            }
        }
        Err(TotpError::Invalid)
    }

    /// Verify the current TOTP code.
    pub fn verify_now(&self, code: &str) -> Result<(), TotpError> {
        self.verify(code, current_unix())
    }

    /// Return the base32-encoded secret for QR code generation.
    pub fn base32_secret(&self) -> String {
        encode_base32(&self.secret)
    }
}

/// Compute a TOTP code for a given counter.
fn compute_totp(secret: &[u8], counter: u64, digits: u32) -> String {
    let mut mac = HmacSha1::new_from_slice(secret).unwrap_or_else(|_| unreachable!("HMAC accepts any key length"));
    mac.update(&counter.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    let truncated: u32 = ((u32::from(hash[offset]) & 0x7f) << 24)
        | (u32::from(hash[offset + 1]) << 16)
        | (u32::from(hash[offset + 2]) << 8)
        | u32::from(hash[offset + 3]);
    let code = truncated % 10u32.pow(digits);
    let width = digits as usize;
    format!("{code:0width$}")
}

fn current_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// RFC 4648 base32 encoding (A-Z2-7).
fn encode_base32(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut result = String::new();
    let mut buffer: u32 = 0;
    let mut bits_left = 0;
    for &byte in data {
        buffer = (buffer << 8) | u32::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            let idx = ((buffer >> (bits_left - 5)) & 0x1f) as usize;
            result.push(ALPHABET[idx] as char);
            bits_left -= 5;
        }
    }
    if bits_left > 0 {
        let idx = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        result.push(ALPHABET[idx] as char);
    }
    result
}

/// RFC 4648 base32 decoding (A-Z2-7, case-insensitive).
#[allow(clippy::cast_possible_truncation)]
fn decode_base32(s: &str) -> Option<Vec<u8>> {
    let mut buffer: u32 = 0;
    let mut bits_left = 0;
    let mut result = Vec::new();
    for ch in s.chars() {
        let val = match ch {
            'A'..='Z' => (ch as u32) - ('A' as u32),
            'a'..='z' => (ch as u32) - ('a' as u32),
            '2'..='7' => (ch as u32) - ('2' as u32) + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | val;
        bits_left += 5;
        if bits_left >= 8 {
            bits_left -= 8;
            result.push((buffer >> bits_left) as u8);
        }
    }
    Some(result)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn totp_generates_6_digit_code() {
        let (_, totp) = Totp::generate();
        let code = totp.now();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn totp_verify_accepts_correct_code() {
        let (_, totp) = Totp::generate();
        let now = current_unix();
        let code = totp.code_at(now);
        totp.verify(&code, now).unwrap();
    }

    #[test]
    fn totp_verify_rejects_wrong_code() {
        let (_, totp) = Totp::generate();
        let err = totp.verify("000000", current_unix()).unwrap_err();
        assert!(matches!(err, TotpError::Invalid));
    }

    #[test]
    fn totp_accepts_within_window() {
        let (_, totp) = Totp::generate();
        let now = current_unix();
        let prev_counter = (now / STEP_SECONDS - 1) * STEP_SECONDS;
        let prev_code = totp.code_at(prev_counter);
        totp.verify(&prev_code, now).unwrap();
    }

    #[test]
    fn totp_rejects_outside_window() {
        let (_, totp) = Totp::generate();
        let now = current_unix();
        let far_counter = (now / STEP_SECONDS - 3) * STEP_SECONDS;
        let far_code = totp.code_at(far_counter);
        assert!(totp.verify(&far_code, now).is_err());
    }

    #[test]
    fn totp_roundtrip_base32() {
        let (b32, totp) = Totp::generate();
        let totp2 = Totp::from_base32(&b32).unwrap();
        let now = current_unix();
        assert_eq!(totp.code_at(now), totp2.code_at(now));
    }

    #[test]
    fn totp_invalid_base32_rejected() {
        assert!(Totp::from_base32("!!!invalid").is_err());
    }

    #[test]
    fn totp_code_changes_over_time() {
        let (_, totp) = Totp::generate();
        let counter = 1_000_000u64;
        let code1 = totp.code_at(counter * STEP_SECONDS);
        let code2 = totp.code_at((counter + 2) * STEP_SECONDS);
        // Very likely different (1/1M chance of collision)
        assert_ne!(code1, code2);
    }

    #[test]
    fn totp_rfc6238_test_vector() {
        // RFC 6238 test vector: secret "12345678901234567890"
        // Key = ASCII bytes of that string
        let secret = b"12345678901234567890";
        let b32 = encode_base32(secret);
        let totp = Totp::from_base32(&b32).unwrap();
        // T=59 -> counter=1 -> expected 287082
        let code = totp.code_at(59);
        assert_eq!(code, "287082");
        // T=1111111109 -> expected 081804
        let code2 = totp.code_at(1_111_111_109);
        assert_eq!(code2, "081804");
    }
}
