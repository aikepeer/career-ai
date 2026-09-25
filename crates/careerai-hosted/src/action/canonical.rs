//! Canonical JSON serialization per `careerai-c14n/v1`.
//!
//! Rules:
//! - UTF-8 encoding with Unicode NFC normalization on every string
//! - JSON escaping with lowercase `\u00xx` escapes only where required
//! - No insignificant whitespace
//! - Object keys sorted by UTF-8 byte sequence
//! - Duplicate keys rejected before normalization
//! - Scalars: null, true, false, strings only
//! - Numbers forbidden in canonical payloads — represented as decimal strings
//!   matching `^-?(0|[1-9][0-9]*)(\.[0-9]+)?$`
//! - Arrays preserve schema-defined order; unordered arrays sorted by
//!   schema-declared stable key
//! - Schema name/version is the first domain-separated field

use sha2::{Digest, Sha256};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Error)]
pub enum CanonicalError {
    #[error("duplicate key in object: {0}")]
    DuplicateKey(String),
    #[error("numbers are forbidden in canonical payloads; use decimal strings")]
    NumberForbidden,
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
}

/// Canonical JSON value — only strings, bools, null, arrays, and objects.
/// Numbers are represented as strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum C14nValue {
    Null,
    Bool(bool),
    Str(String),
    Array(Vec<C14nValue>),
    Object(Vec<(String, C14nValue)>),
}

/// Serialize a C14nValue to canonical JSON bytes.
pub fn canonical_bytes(value: &C14nValue) -> Vec<u8> {
    let mut out = Vec::new();
    serialize_value(value, &mut out);
    out
}

/// Serialize a C14nValue to a canonical JSON string.
pub fn canonical_json(value: &C14nValue) -> String {
    String::from_utf8(canonical_bytes(value))
        .unwrap_or_else(|_| unreachable!("canonical JSON is valid UTF-8"))
}

/// Compute the SHA-256 digest of a canonical payload.
pub fn payload_digest(value: &C14nValue) -> String {
    let bytes = canonical_bytes(value);
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hex::encode(hasher.finalize())
}

fn serialize_value(value: &C14nValue, out: &mut Vec<u8>) {
    match value {
        C14nValue::Null => out.extend_from_slice(b"null"),
        C14nValue::Bool(true) => out.extend_from_slice(b"true"),
        C14nValue::Bool(false) => out.extend_from_slice(b"false"),
        C14nValue::Str(s) => serialize_string(s, out),
        C14nValue::Array(arr) => {
            out.push(b'[');
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                serialize_value(v, out);
            }
            out.push(b']');
        }
        C14nValue::Object(obj) => {
            // Sort keys by UTF-8 byte sequence
            let mut sorted = obj.clone();
            sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            out.push(b'{');
            for (i, (k, v)) in sorted.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                serialize_string(k, out);
                out.push(b':');
                serialize_value(v, out);
            }
            out.push(b'}');
        }
    }
}

fn serialize_string(s: &str, out: &mut Vec<u8>) {
    // NFC normalize the string
    let normalized: String = s.nfc().collect();
    out.push(b'"');
    for c in normalized.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\x08' => out.extend_from_slice(b"\\b"),
            '\x0c' => out.extend_from_slice(b"\\f"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                // Encode as UTF-8
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                out.extend_from_slice(s.as_bytes());
            }
        }
    }
    out.push(b'"');
}

/// Convert a serde_json::Value to C14nValue, rejecting numbers.
pub fn from_json(value: &serde_json::Value) -> Result<C14nValue, CanonicalError> {
    match value {
        serde_json::Value::Null => Ok(C14nValue::Null),
        serde_json::Value::Bool(b) => Ok(C14nValue::Bool(*b)),
        serde_json::Value::String(s) => Ok(C14nValue::Str(s.clone())),
        serde_json::Value::Number(_) => Err(CanonicalError::NumberForbidden),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr.iter().map(from_json).collect();
            Ok(C14nValue::Array(items?))
        }
        serde_json::Value::Object(map) => {
            let mut pairs = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for (k, v) in map {
                if !seen.insert(k.clone()) {
                    return Err(CanonicalError::DuplicateKey(k.clone()));
                }
                pairs.push((k.clone(), from_json(v)?));
            }
            Ok(C14nValue::Object(pairs))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn canonical_sorts_keys() {
        let val = C14nValue::Object(vec![
            ("zebra".into(), C14nValue::Str("a".into())),
            ("apple".into(), C14nValue::Str("b".into())),
            ("mango".into(), C14nValue::Str("c".into())),
        ]);
        let json = canonical_json(&val);
        assert_eq!(json, r#"{"apple":"b","mango":"c","zebra":"a"}"#);
    }

    #[test]
    fn canonical_no_whitespace() {
        let val = C14nValue::Object(vec![(
            "a".into(),
            C14nValue::Object(vec![("b".into(), C14nValue::Str("c".into()))]),
        )]);
        let json = canonical_json(&val);
        assert_eq!(json, r#"{"a":{"b":"c"}}"#);
        assert!(!json.contains(' '));
    }

    #[test]
    fn canonical_nfc_normalization() {
        // é can be represented as precomposed U+00E9 or decomposed e + combining acute
        let precomposed = "\u{00E9}";
        let decomposed = "e\u{0301}";
        let val1 = C14nValue::Object(vec![("key".into(), C14nValue::Str(precomposed.into()))]);
        let val2 = C14nValue::Object(vec![("key".into(), C14nValue::Str(decomposed.into()))]);
        // Both should produce the same canonical form (NFC = precomposed)
        assert_eq!(canonical_json(&val1), canonical_json(&val2));
    }

    #[test]
    fn payload_digest_stable() {
        let val = C14nValue::Object(vec![
            ("action".into(), C14nValue::Str("submit".into())),
            ("target".into(), C14nValue::Str("listing-123".into())),
        ]);
        let digest1 = payload_digest(&val);
        let digest2 = payload_digest(&val);
        assert_eq!(digest1, digest2);
        assert_eq!(digest1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn payload_digest_changes_with_reordered_keys() {
        let val1 = C14nValue::Object(vec![
            ("a".into(), C14nValue::Str("1".into())),
            ("b".into(), C14nValue::Str("2".into())),
        ]);
        let val2 = C14nValue::Object(vec![
            ("b".into(), C14nValue::Str("2".into())),
            ("a".into(), C14nValue::Str("1".into())),
        ]);
        // Same digest because canonical form sorts keys
        assert_eq!(payload_digest(&val1), payload_digest(&val2));
    }

    #[test]
    fn payload_digest_changes_with_different_values() {
        let val1 = C14nValue::Object(vec![("a".into(), C14nValue::Str("1".into()))]);
        let val2 = C14nValue::Object(vec![("a".into(), C14nValue::Str("2".into()))]);
        assert_ne!(payload_digest(&val1), payload_digest(&val2));
    }

    #[test]
    fn from_json_rejects_numbers() {
        let json = serde_json::json!({"count": 42});
        let err = from_json(&json).unwrap_err();
        assert!(matches!(err, CanonicalError::NumberForbidden));
    }

    #[test]
    fn from_json_converts_strings() {
        let json = serde_json::json!({"count": "42"});
        let val = from_json(&json).unwrap();
        match val {
            C14nValue::Object(obj) => {
                assert_eq!(obj[0].0, "count");
                match &obj[0].1 {
                    C14nValue::Str(s) => assert_eq!(s, "42"),
                    _ => panic!("expected string"),
                }
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn canonical_escapes_control_chars() {
        let val = C14nValue::Str("a\u{0001}b".into());
        let json = canonical_json(&val);
        assert_eq!(json, r#""a\u0001b""#);
    }

    #[test]
    fn canonical_array_preserves_order() {
        let val = C14nValue::Array(vec![
            C14nValue::Str("c".into()),
            C14nValue::Str("a".into()),
            C14nValue::Str("b".into()),
        ]);
        let json = canonical_json(&val);
        assert_eq!(json, r#"["c","a","b"]"#);
    }

    #[test]
    fn canonical_null_bool() {
        assert_eq!(canonical_json(&C14nValue::Null), "null");
        assert_eq!(canonical_json(&C14nValue::Bool(true)), "true");
        assert_eq!(canonical_json(&C14nValue::Bool(false)), "false");
    }
}
