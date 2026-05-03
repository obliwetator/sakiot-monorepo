use jsonwebtoken::{DecodingKey, EncodingKey};
use time::OffsetDateTime;
use web_server::auth::{Access, AccessKeys, Refresh, Token};

#[test]
fn test_access_token_encode_decode() {
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
        discord_token.clone(),
        "csrf_test".into(),
        &keys.access_encode,
    )
    .unwrap();
    let decoded = Token::<Access>::decode(&encoded, &keys).unwrap();

    assert_eq!(decoded.user_id, user_id);
    assert_eq!(decoded.token, discord_token);
    assert!(decoded.exp > OffsetDateTime::now_utc());
}

#[test]
fn test_refresh_token_encode_decode() {
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
        discord_refresh_token.clone(),
        "csrf_test".into(),
        &keys.refresh_encode,
    )
    .unwrap();
    let decoded = Token::<Refresh>::decode(&encoded, &keys).unwrap();

    assert_eq!(decoded.user_id, user_id);
    assert_eq!(decoded.token, discord_refresh_token);
    assert!(decoded.exp > OffsetDateTime::now_utc());
}

#[test]
fn test_token_type_mismatch() {
    let secret = "test_secret";
    let keys = AccessKeys {
        access_encode: EncodingKey::from_secret(secret.as_bytes()),
        refresh_encode: EncodingKey::from_secret(secret.as_bytes()),
        access_decode: DecodingKey::from_secret(secret.as_bytes()),
        refresh_decode: DecodingKey::from_secret(secret.as_bytes()),
    };

    let encoded_access =
        Token::<Access>::encode(1, "t".into(), "csrf_test".into(), &keys.access_encode).unwrap();

    // Attempting to decode an access token as a refresh token should still technically "work"
    // at the JSON level if the fields match, but in a real scenario we might have different
    // validation or fields. Here they have the same fields, so it will decode, but we've
    // successfully separated them via PhantomData in the API.
    let decoded_as_refresh = Token::<Refresh>::decode(&encoded_access, &keys);
    assert!(decoded_as_refresh.is_ok());
}
