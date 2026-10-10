use jsonwebtoken::{EncodingKey, Header, encode};
use serde_json::{Value, json};

use super::*;

const KEY_A: &str = include_str!("../../../tests/fixtures/oidc/key_a.pem");
const KEY_B: &str = include_str!("../../../tests/fixtures/oidc/key_b.pem");
const JWK_A: &str = include_str!("../../../tests/fixtures/oidc/jwk_a.json");
const ISS: &str = "https://accounts.google.com";
const AUD: &str = "client-123.apps.example";
const NONCE: &str = "n0nce";

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn keys() -> JwkSet {
    let jwk: Value = serde_json::from_str(JWK_A).expect("jwk");
    serde_json::from_value(json!({ "keys": [jwk] })).expect("jwks")
}

fn claims() -> Value {
    json!({
        "iss": ISS,
        "aud": AUD,
        "sub": "1234567890",
        "iat": now(),
        "exp": now() + 3600,
        "nonce": NONCE,
        "email": "Jan.Novak@Example.com",
        "email_verified": true,
        "name": "Jan Novák",
    })
}

fn sign_with(pem: &str, kid: Option<&str>, claims: &Value) -> String {
    let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = kid.map(str::to_string);
    let key = EncodingKey::from_rsa_pem(pem.as_bytes()).expect("test key");
    encode(&header, claims, &key).expect("sign")
}

fn sign(claims: &Value) -> String {
    sign_with(KEY_A, Some("test-a"), claims)
}

fn check(token: &str) -> Result<Identity, Rejected> {
    let expected = Expected {
        issuer: ISS,
        client_id: AUD,
        nonce: NONCE,
    };
    verify(token, &keys(), &expected, now())
}

fn with(key: &str, value: Value) -> Value {
    let mut c = claims();
    c[key] = value;
    c
}

fn without(key: &str) -> Value {
    let mut c = claims();
    c.as_object_mut().expect("object").remove(key);
    c
}

fn is_invalid(r: Result<Identity, Rejected>) -> bool {
    matches!(r, Err(Rejected::Invalid(_)))
}

#[test]
fn accepts_a_valid_token() {
    let id = check(&sign(&claims())).expect("valid");
    assert_eq!(
        id,
        Identity {
            sub: "1234567890".into(),
            email: "jan.novak@example.com".into(),
            name: Some("Jan Novák".into()),
        }
    );
    // The scheme-less issuer Google also uses, an audience list, a string flag.
    let mut c = with("iss", json!("accounts.google.com"));
    c["aud"] = json!(["other", AUD]);
    c["azp"] = json!(AUD);
    c["email_verified"] = json!("true");
    assert!(check(&sign(&c)).is_ok());
    assert!(check(&sign(&with("aud", json!([AUD])))).is_ok());
}

#[test]
fn signature_and_key() {
    // Signed with another key under the known kid.
    assert!(is_invalid(check(&sign_with(
        KEY_B,
        Some("test-a"),
        &claims()
    ))));
    assert_eq!(
        check(&sign_with(KEY_A, Some("test-b"), &claims())),
        Err(Rejected::UnknownKid)
    );
    assert!(is_invalid(check(&sign_with(KEY_A, None, &claims()))));
    // Tamper with the signature: a different character well inside it (the
    // last one may carry only padding bits).
    let token = sign(&claims());
    let at = token.rfind('.').expect("signature part") + 10;
    let swapped = if &token[at..=at] == "A" { "B" } else { "A" };
    let token = format!("{}{swapped}{}", &token[..at], &token[at + 1..]);
    assert!(is_invalid(check(&token)));
    let hs = encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        &claims(),
        &EncodingKey::from_secret(b"secret"),
    )
    .expect("sign");
    assert!(is_invalid(check(&hs)));
    assert!(is_invalid(check("not.a.token")));
}

#[test]
fn issuer_and_audience() {
    assert!(is_invalid(check(&sign(&with(
        "iss",
        json!("https://evil.example.com")
    )))));
    assert!(is_invalid(check(&sign(&with(
        "aud",
        json!("other-client")
    )))));
    assert!(is_invalid(check(&sign(&without("aud")))));
    assert!(is_invalid(check(&sign(&without("iss")))));
}

#[test]
fn authorized_party() {
    assert!(check(&sign(&with("azp", json!(AUD)))).is_ok());
    assert!(is_invalid(check(&sign(&with(
        "azp",
        json!("other-client")
    )))));
    // Several audiences: `azp` required and ours.
    let mut c = with("aud", json!([AUD, "other"]));
    assert!(is_invalid(check(&sign(&c))));
    c["azp"] = json!("other");
    assert!(is_invalid(check(&sign(&c))));
    c["azp"] = json!(AUD);
    assert!(check(&sign(&c)).is_ok());
}

#[test]
fn time_claims() {
    assert!(is_invalid(check(&sign(&with("exp", json!(now() - 120))))));
    // Within the 60 s skew.
    assert!(check(&sign(&with("exp", json!(now() - 30)))).is_ok());
    assert!(check(&sign(&with("iat", json!(now() + 30)))).is_ok());
    assert!(is_invalid(check(&sign(&with("iat", json!(now() + 120))))));
    assert!(is_invalid(check(&sign(&without("iat")))));
    assert!(is_invalid(check(&sign(&without("exp")))));
}

#[test]
fn nonce() {
    assert!(is_invalid(check(&sign(&with("nonce", json!("other"))))));
    assert!(is_invalid(check(&sign(&without("nonce")))));
}

#[test]
fn email_must_be_verified() {
    for flag in [json!(false), json!("false"), json!(null)] {
        assert_eq!(
            check(&sign(&with("email_verified", flag))),
            Err(Rejected::UnverifiedEmail)
        );
    }
    assert_eq!(
        check(&sign(&without("email_verified"))),
        Err(Rejected::UnverifiedEmail)
    );
    assert!(is_invalid(check(&sign(&without("email")))));
    assert!(is_invalid(check(&sign(&with("sub", json!(""))))));
    let id = check(&sign(&with("name", json!("  ")))).expect("valid");
    assert_eq!(id.name, None);
}
