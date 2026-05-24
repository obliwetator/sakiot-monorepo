use jsonwebtoken::{DecodingKey, EncodingKey};
use time::OffsetDateTime;
use web_server::auth::{Access, AccessKeys, AuthKind, Refresh, Token};

#[test]
fn test_access_token_encode_decode() -> Result<(), Box<dyn std::error::Error>> {
    let secret = "test_secret";
    let keys = AccessKeys {
        access_encode: EncodingKey::from_secret(secret.as_bytes()),
        refresh_encode: EncodingKey::from_secret(secret.as_bytes()),
        access_decode: DecodingKey::from_secret(secret.as_bytes()),
        refresh_decode: DecodingKey::from_secret(secret.as_bytes()),
    };

    let user_id = 12345;
    let discord_token = "discord_abc".to_string();

    let encoded = Token::<Access>::encode(
        user_id,
        AuthKind::Discord,
        "csrf_test".into(),
        &keys.access_encode,
    )?;
    let decoded = Token::<Access>::decode(&encoded, &keys)?;

    assert_eq!(decoded.user_id, user_id);
    assert_eq!(decoded.auth_kind, AuthKind::Discord);
    assert!(decoded.exp > OffsetDateTime::now_utc());
    assert_token_payload_does_not_contain(&encoded, &discord_token)?;
    Ok(())
}

#[test]
fn test_refresh_token_encode_decode() -> Result<(), Box<dyn std::error::Error>> {
    let secret = "test_secret";
    let keys = AccessKeys {
        access_encode: EncodingKey::from_secret(secret.as_bytes()),
        refresh_encode: EncodingKey::from_secret(secret.as_bytes()),
        access_decode: DecodingKey::from_secret(secret.as_bytes()),
        refresh_decode: DecodingKey::from_secret(secret.as_bytes()),
    };

    let user_id = 12345;
    let discord_refresh_token = "discord_refresh_abc".to_string();

    let encoded = Token::<Refresh>::encode(
        user_id,
        AuthKind::Discord,
        "csrf_test".into(),
        &keys.refresh_encode,
    )?;
    let decoded = Token::<Refresh>::decode(&encoded, &keys)?;

    assert_eq!(decoded.user_id, user_id);
    assert_eq!(decoded.auth_kind, AuthKind::Discord);
    assert!(decoded.exp > OffsetDateTime::now_utc());
    assert_token_payload_does_not_contain(&encoded, &discord_refresh_token)?;
    Ok(())
}

#[test]
fn test_token_type_mismatch() -> Result<(), Box<dyn std::error::Error>> {
    let secret = "test_secret";
    let keys = AccessKeys {
        access_encode: EncodingKey::from_secret(secret.as_bytes()),
        refresh_encode: EncodingKey::from_secret(secret.as_bytes()),
        access_decode: DecodingKey::from_secret(secret.as_bytes()),
        refresh_decode: DecodingKey::from_secret(secret.as_bytes()),
    };

    let encoded_access = Token::<Access>::encode(
        1,
        AuthKind::Discord,
        "csrf_test".into(),
        &keys.access_encode,
    )?;

    let decoded_as_refresh = Token::<Refresh>::decode(&encoded_access, &keys);
    assert!(decoded_as_refresh.is_err());
    Ok(())
}

fn assert_token_payload_does_not_contain(
    token: &str,
    needle: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let payload = token
        .split('.')
        .nth(1)
        .ok_or("jwt payload segment missing")?;
    let decoded = URL_SAFE_NO_PAD.decode(payload)?;
    let payload = String::from_utf8(decoded)?;

    assert!(
        !payload.contains(needle),
        "JWT payload should not contain legacy Discord token"
    );
    Ok(())
}
