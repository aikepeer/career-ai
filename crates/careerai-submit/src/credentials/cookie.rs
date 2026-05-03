use super::credential::{self, Credential};

pub fn cookie_expiry(provider: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let key = match provider {
        "linkedin" => "li_at",
        _ => return None,
    };
    let cred = Credential::for_source(provider, key);
    let token = credential::load(&cred).ok()?;
    parse_jwt_exp(&token)
}

/// Parse the `exp` claim out of a JWT string.
#[must_use]
pub fn parse_jwt_exp(token: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use base64::Engine;

    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let _sig = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let exp = json.get("exp")?.as_i64()?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(exp, 0)
}

#[must_use]
pub fn cookie_remaining(provider: &str) -> Option<chrono::Duration> {
    let exp = cookie_expiry(provider)?;
    Some(exp - chrono::Utc::now())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieHealth {
    NotStored,
    Unparseable,
    Expired(chrono::Duration),
    ExpiringSoon(chrono::Duration),
    Healthy(chrono::Duration),
}

#[must_use]
pub fn cookie_health(provider: &str) -> CookieHealth {
    let key = match provider {
        "linkedin" => "li_at",
        _ => return CookieHealth::NotStored,
    };
    let cred = Credential::for_source(provider, key);
    let Ok(token) = credential::load(&cred) else {
        return CookieHealth::NotStored;
    };
    let Some(exp) = parse_jwt_exp(&token) else {
        return CookieHealth::Unparseable;
    };
    let now = chrono::Utc::now();
    if exp <= now {
        return CookieHealth::Expired(now - exp);
    }
    let remaining = exp - now;
    if remaining < chrono::Duration::hours(48) {
        CookieHealth::ExpiringSoon(remaining)
    } else {
        CookieHealth::Healthy(remaining)
    }
}
