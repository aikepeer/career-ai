#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn env_var_name_uppercases_with_underscores() {
    let c = Credential::for_source("linkedin", "li_at");
    assert_eq!(c.env_var_name(), "CAREERAI_LINKEDIN_LI_AT");
    let c = Credential::for_source("indeed", "session-cookie");
    assert_eq!(c.env_var_name(), "CAREERAI_INDEED_SESSION_COOKIE");
}

#[test]
fn keyring_username_is_namespaced_per_source() {
    let c = Credential::for_source("linkedin", "li_at");
    assert_eq!(c.keyring_username(), "linkedin/li_at");
}

fn fake_jwt_with_exp(exp_secs: i64) -> String {
    use base64::Engine;
    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload_json = format!(r#"{{"sub":"test","exp":{exp_secs}}}"#);
    let payload =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"unverified");
    format!("{header}.{payload}.{sig}")
}

#[test]
fn parse_jwt_exp_decodes_valid_jwt() {
    let exp = 2_000_000_000_i64;
    let jwt = fake_jwt_with_exp(exp);
    let parsed = parse_jwt_exp(&jwt).expect("valid JWT must decode");
    assert_eq!(parsed.timestamp(), exp);
}

#[test]
fn parse_jwt_exp_rejects_non_jwt_shapes() {
    assert!(parse_jwt_exp("opaque-cookie").is_none());
    assert!(parse_jwt_exp("a.b").is_none());
    assert!(parse_jwt_exp("a.b.c.d").is_none());
}

#[test]
fn parse_jwt_exp_rejects_unparseable_payload() {
    use base64::Engine;
    assert!(parse_jwt_exp("header.notbase64!@#.sig").is_none());
    let bad_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"not json");
    assert!(parse_jwt_exp(&format!("h.{bad_payload}.s")).is_none());
}

#[test]
fn parse_jwt_exp_rejects_payload_without_exp_claim() {
    use base64::Engine;
    let json = r#"{"sub":"test"}"#;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes());
    assert!(parse_jwt_exp(&format!("h.{payload}.s")).is_none());
}

#[test]
fn cookie_expiry_short_circuits_for_non_linkedin_provider() {
    assert!(cookie_expiry("naukri").is_none());
    assert!(cookie_expiry("indeed").is_none());
    assert_eq!(cookie_health("naukri"), CookieHealth::NotStored);
    assert_eq!(cookie_health("indeed"), CookieHealth::NotStored);
}

#[test]
fn cookie_expiry_handles_linkedin_lookup_without_panicking() {
    let _ = cookie_expiry("linkedin");
    let _ = cookie_remaining("linkedin");
    let _ = cookie_health("linkedin");
}

#[test]
fn missing_credential_error_is_actionable() {
    let c = Credential::for_source("__nonexistent_test_source__", "__nope__");
    let err = load(&c).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("keychain"), "got: {msg}");
    assert!(msg.contains("CAREERAI_"), "got: {msg}");
    assert!(!msg.contains("password"), "got: {msg}");
}
