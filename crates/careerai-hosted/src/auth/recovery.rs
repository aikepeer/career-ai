//! Recovery codes: shown once, hashed at rest, regeneration invalidates prior set.
//!
//! Recovery codes are 10 codes per set, each 20 characters (4 groups of 5).
//! They are hashed with SHA-256 and shown only once. Regenerating a set
//! atomically invalidates the prior set. Recovery requires verified email
//! plus an unused recovery code.

use constant_time_eq::constant_time_eq;
use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;

const CODE_COUNT: usize = 10;
const CODE_BYTES: usize = 15; // 20 base32 chars

#[derive(Debug, Error)]
pub enum RecoveryCodeError {
    #[error("recovery code has already been used")]
    AlreadyUsed,
    #[error("recovery code does not match")]
    Invalid,
    #[error("recovery code set has been invalidated")]
    Invalidated,
}

/// A single recovery code record (stored hashed).
#[derive(Debug, Clone)]
pub struct RecoveryCodeRecord {
    pub code_hash: String,
    pub used: bool,
}

/// A recovery code set (stored hashed, shown once).
#[derive(Debug, Clone)]
pub struct RecoveryCodeSet {
    /// Unique set identifier.
    pub set_id: String,
    /// Hashed codes.
    pub codes: Vec<RecoveryCodeRecord>,
    /// Whether this set has been invalidated by regeneration.
    pub invalidated: bool,
}

impl RecoveryCodeSet {
    /// Generate a new recovery code set.
    /// Returns (raw_codes, set) where raw_codes are shown once to the user.
    pub fn generate() -> (Vec<String>, Self) {
        let set_id = generate_set_id();
        let mut raw_codes = Vec::with_capacity(CODE_COUNT);
        let mut records = Vec::with_capacity(CODE_COUNT);
        for _ in 0..CODE_COUNT {
            let mut bytes = [0u8; CODE_BYTES];
            rand::thread_rng().fill_bytes(&mut bytes);
            let raw = format_recovery_code(&bytes);
            let normalized = raw.replace('-', "");
            let hash = hash_code(&normalized);
            raw_codes.push(raw);
            records.push(RecoveryCodeRecord {
                code_hash: hash,
                used: false,
            });
        }
        let set = Self {
            set_id,
            codes: records,
            invalidated: false,
        };
        (raw_codes, set)
    }

    /// Verify a recovery code against the set.
    /// Marks the code as used if valid (single-use).
    pub fn verify(&mut self, raw_code: &str) -> Result<(), RecoveryCodeError> {
        if self.invalidated {
            return Err(RecoveryCodeError::Invalidated);
        }
        let normalized = raw_code.replace('-', "");
        let provided_hash = hash_code(&normalized);
        for code in &mut self.codes {
            if !code.used && constant_time_eq(provided_hash.as_bytes(), code.code_hash.as_bytes()) {
                code.used = true;
                return Ok(());
            }
        }
        // Check if it was already used (don't reveal which codes exist)
        for code in &self.codes {
            if constant_time_eq(provided_hash.as_bytes(), code.code_hash.as_bytes()) {
                return Err(RecoveryCodeError::AlreadyUsed);
            }
        }
        Err(RecoveryCodeError::Invalid)
    }

    /// Check if all codes have been used.
    pub fn all_used(&self) -> bool {
        self.codes.iter().all(|c| c.used)
    }

    /// Invalidate this set (called when a new set is generated).
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }
}

fn hash_code(code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    hex::encode(hasher.finalize())
}

fn generate_set_id() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Format raw bytes as a recovery code: XXXXX-XXXXX-XXXXX-XXXXX
fn format_recovery_code(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut result = String::new();
    let mut buffer: u32 = 0;
    let mut bits_left = 0;
    let mut group_count = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            let idx = ((buffer >> (bits_left - 5)) & 0x1f) as usize;
            result.push(ALPHABET[idx] as char);
            bits_left -= 5;
            group_count += 1;
            if group_count % 5 == 0 && group_count < 20 {
                result.push('-');
            }
        }
    }
    if bits_left > 0 {
        let idx = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        result.push(ALPHABET[idx] as char);
    }
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_10_codes() {
        let (codes, _set) = RecoveryCodeSet::generate();
        assert_eq!(codes.len(), 10);
    }

    #[test]
    fn codes_are_unique() {
        let (codes, _) = RecoveryCodeSet::generate();
        let set: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(set.len(), 10);
    }

    #[test]
    fn verify_accepts_correct_code() {
        let (codes, mut set) = RecoveryCodeSet::generate();
        set.verify(&codes[0]).unwrap();
    }

    #[test]
    fn verify_rejects_used_code() {
        let (codes, mut set) = RecoveryCodeSet::generate();
        set.verify(&codes[0]).unwrap();
        let err = set.verify(&codes[0]).unwrap_err();
        assert!(matches!(err, RecoveryCodeError::AlreadyUsed));
    }

    #[test]
    fn verify_rejects_wrong_code() {
        let (_, mut set) = RecoveryCodeSet::generate();
        let err = set.verify("WRONG-CODE-VALUE").unwrap_err();
        assert!(matches!(err, RecoveryCodeError::Invalid));
    }

    #[test]
    fn invalidated_set_rejects_all() {
        let (codes, mut set) = RecoveryCodeSet::generate();
        set.invalidate();
        let err = set.verify(&codes[0]).unwrap_err();
        assert!(matches!(err, RecoveryCodeError::Invalidated));
    }

    #[test]
    fn all_used_detects_exhausted_set() {
        let (codes, mut set) = RecoveryCodeSet::generate();
        for code in &codes {
            set.verify(code).unwrap();
        }
        assert!(set.all_used());
    }

    #[test]
    fn different_sets_have_different_ids() {
        let (_, set1) = RecoveryCodeSet::generate();
        let (_, set2) = RecoveryCodeSet::generate();
        assert_ne!(set1.set_id, set2.set_id);
    }

    #[test]
    fn code_hashes_not_in_raw() {
        let (codes, set) = RecoveryCodeSet::generate();
        for (raw, record) in codes.iter().zip(set.codes.iter()) {
            assert!(!record.code_hash.contains(raw));
        }
    }

    #[test]
    fn code_format_has_dashes() {
        let (codes, _) = RecoveryCodeSet::generate();
        assert!(codes[0].contains('-'));
    }
}
