//! The two short-lived rows of a Google sign-in: the **flow** (10 min,
//! single use, keyed by sha256 of the OAuth `state`) and the **handoff**
//! (60 s, single use, sha256 of its token, bound to the origin host). Also
//! PKCE and the `return` path rule.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::entity::{google_flow, google_handoff};
use super::idtoken::Identity;
use crate::auth::crypto;
use crate::auth::mfa::seal;
use crate::error::AppError;
use crate::secret::SecretKey;
use crate::space::SpaceId;

pub const FLOW_TTL: Duration = Duration::minutes(10);
pub const HANDOFF_TTL: Duration = Duration::seconds(60);
const RETURN_MAX: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Login,
    Link,
    Invite,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Link => "link",
            Self::Invite => "invite",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "login" => Some(Self::Login),
            "link" => Some(Self::Link),
            "invite" => Some(Self::Invite),
            _ => None,
        }
    }
}

/// PKCE S256: base64url(sha256(verifier)) without padding.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// `return` is kept only as a relative path on the same host (`/…`, not
/// `//…` or `/\…`, printable ASCII without spaces or backslashes, at most
/// 512 bytes, not under `/api`); anything else becomes `/`.
pub fn sanitize_return(raw: Option<&str>) -> String {
    raw.filter(|p| {
        p.len() <= RETURN_MAX
            && p.starts_with('/')
            && !p.starts_with("//")
            && p.bytes().all(|b| (0x21..=0x7e).contains(&b) && b != b'\\')
            && !(p == &"/api" || p.starts_with("/api/") || p.starts_with("/api?"))
    })
    .map_or_else(|| "/".to_string(), str::to_string)
}

/// An invitation token as it may appear in a redirect URL (base64url).
pub fn url_safe_token(raw: &str) -> Option<&str> {
    (!raw.is_empty()
        && raw.len() <= 128
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    .then_some(raw)
}

/// What `start` records.
pub struct NewFlow {
    pub space: Option<SpaceId>,
    pub purpose: Purpose,
    pub return_path: String,
    /// `invite`: the raw invitation token.
    pub invite: Option<String>,
    /// `link`: the signed-in user.
    pub user_id: Option<Uuid>,
    /// The value of the `invoice_google` cookie set on the starting browser.
    pub browser: String,
}

/// The raw values that leave in the authorization request, and the name of
/// this flow's browser-binding cookie.
pub struct Started {
    pub state: String,
    pub verifier: String,
    pub nonce: String,
    pub cookie_name: String,
}

/// `invoice_google_{12 hex}`: one cookie per flow, so parallel sign-ins in
/// one browser (two tabs, two hosts) do not overwrite each other's binding.
pub fn cookie_name(flow_id: Uuid) -> String {
    format!(
        "{}_{}",
        super::BROWSER_COOKIE,
        &flow_id.simple().to_string()[..12]
    )
}

/// The invitation token sealed for the error redirect (associated data =
/// the space, so a sealed value is useless for another space).
fn seal_invite(key: &SecretKey, space: Option<SpaceId>, token: &str) -> Result<Vec<u8>, AppError> {
    let space = space.ok_or(AppError::NotFound)?;
    seal::seal(key, space.uuid(), token.as_bytes())
}

pub fn open_invite(key: &SecretKey, space: Option<Uuid>, sealed: Option<&[u8]>) -> Option<String> {
    let plain = seal::open(key, space?, sealed?).ok()?;
    String::from_utf8(plain).ok()
}

/// Store a new flow; expired flows and handoffs of anyone are purged on
/// the way.
pub async fn create(
    db: &DatabaseConnection,
    key: &SecretKey,
    new: NewFlow,
) -> Result<Started, AppError> {
    let now = Utc::now();
    google_flow::Entity::delete_many()
        .filter(google_flow::Column::ExpiresAt.lte(now))
        .exec(db)
        .await?;
    google_handoff::Entity::delete_many()
        .filter(google_handoff::Column::ExpiresAt.lte(now))
        .exec(db)
        .await?;
    let id = Uuid::new_v4();
    let started = Started {
        state: crypto::random_token()?,
        verifier: crypto::random_token()?,
        nonce: crypto::random_token()?,
        cookie_name: cookie_name(id),
    };
    let invite_sealed = match &new.invite {
        Some(t) => Some(seal_invite(key, new.space, t)?),
        None => None,
    };
    let now: DateTime<FixedOffset> = now.into();
    google_flow::ActiveModel {
        id: Set(id),
        state_hash: Set(crypto::digest(&started.state)),
        space_id: Set(new.space.map(SpaceId::uuid)),
        purpose: Set(new.purpose.as_str().to_string()),
        pkce_verifier: Set(started.verifier.clone()),
        nonce: Set(started.nonce.clone()),
        return_path: Set(new.return_path),
        invite_hash: Set(new.invite.as_deref().map(crypto::digest)),
        invite_sealed: Set(invite_sealed),
        user_id: Set(new.user_id),
        browser_cookie: Set(started.cookie_name.clone()),
        browser_hash: Set(crypto::digest(&new.browser)),
        created_at: Set(now),
        expires_at: Set(now + FLOW_TTL),
    }
    .insert(db)
    .await?;
    Ok(started)
}

/// Use up the flow of `state` (expired or not: the caller tells them
/// apart); `None` = unknown or already used.
pub async fn take_flow(
    db: &impl ConnectionTrait,
    state: &str,
) -> Result<Option<google_flow::Model>, AppError> {
    if state.is_empty() {
        return Ok(None);
    }
    let gone = google_flow::Entity::delete_many()
        .filter(google_flow::Column::StateHash.eq(crypto::digest(state)))
        .exec_with_returning(db)
        .await?;
    Ok(gone.into_iter().next())
}

/// The verified claims a handoff carries.
pub fn identity(h: &google_handoff::Model) -> Identity {
    Identity {
        sub: h.google_sub.clone(),
        email: h.google_email.clone(),
        name: h.google_name.clone(),
    }
}

pub fn expired(expires_at: DateTime<FixedOffset>) -> bool {
    expires_at.to_utc() <= Utc::now()
}

/// The callback's result for the origin host — the verified claims; the
/// user is resolved (and any account written) only by `finish`, after the
/// browser binding. Returns the raw token.
pub async fn create_handoff(
    db: &impl ConnectionTrait,
    flow: &google_flow::Model,
    id: &Identity,
) -> Result<String, AppError> {
    let token = crypto::random_token()?;
    let now: DateTime<FixedOffset> = Utc::now().into();
    google_handoff::ActiveModel {
        id: Set(Uuid::new_v4()),
        token_hash: Set(crypto::digest(&token)),
        space_id: Set(flow.space_id),
        user_id: Set(flow.user_id),
        purpose: Set(flow.purpose.clone()),
        return_path: Set(flow.return_path.clone()),
        invite_hash: Set(flow.invite_hash.clone()),
        invite_sealed: Set(flow.invite_sealed.clone()),
        google_sub: Set(id.sub.clone()),
        google_email: Set(id.email.clone()),
        google_name: Set(id.name.clone()),
        browser_cookie: Set(flow.browser_cookie.clone()),
        browser_hash: Set(flow.browser_hash.clone()),
        created_at: Set(now),
        expires_at: Set(now + HANDOFF_TTL),
    }
    .insert(db)
    .await?;
    Ok(token)
}

/// Use up the handoff of `token` **on its own host** (`space` = this host;
/// on another host it is left alone); expired or not.
pub async fn take_handoff(
    db: &impl ConnectionTrait,
    token: &str,
    space: Option<SpaceId>,
) -> Result<Option<google_handoff::Model>, AppError> {
    if token.is_empty() {
        return Ok(None);
    }
    let host = match space {
        Some(s) => google_handoff::Column::SpaceId.eq(s.uuid()),
        None => google_handoff::Column::SpaceId.is_null(),
    };
    let gone = google_handoff::Entity::delete_many()
        .filter(google_handoff::Column::TokenHash.eq(crypto::digest(token)))
        .filter(host)
        .exec_with_returning(db)
        .await?;
    Ok(gone.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256() {
        // RFC 7636, appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let v = crypto::random_token().expect("verifier");
        assert!((43..=128).contains(&v.len()));
        assert_eq!(challenge(&v).len(), 43);
    }

    #[test]
    fn return_paths() {
        for ok in ["/", "/documents", "/documents/1?tab=pdf#x", "/a-b_c.~%20"] {
            assert_eq!(sanitize_return(Some(ok)), ok);
        }
        for bad in [
            None,
            Some(""),
            Some("documents"),
            Some("//evil.example.com"),
            Some("/\\evil.example.com"),
            Some("/x\\y"),
            Some("https://evil.example.com/"),
            Some("/a b"),
            Some("/a\r\nSet-Cookie: x=1"),
            Some("/ž"),
            Some("/api"),
            Some("/api/auth/google/start?purpose=login"),
            Some("/api?x"),
        ] {
            assert_eq!(sanitize_return(bad), "/", "{bad:?}");
        }
        let long = format!("/{}", "a".repeat(RETURN_MAX));
        assert_eq!(sanitize_return(Some(&long)), "/");
        assert_eq!(sanitize_return(Some("/apis")), "/apis");
    }

    #[test]
    fn cookie_names_are_per_flow() {
        let id = Uuid::new_v4();
        let name = cookie_name(id);
        assert!(name.starts_with("invoice_google_"));
        assert_eq!(name.len(), "invoice_google_".len() + 12);
        assert_eq!(name, cookie_name(id));
        assert_ne!(name, cookie_name(Uuid::new_v4()));
    }

    /// The handoff rebuilds the identity from its row.
    #[test]
    fn handoff_identity() {
        let h = google_handoff::Model {
            id: Uuid::new_v4(),
            token_hash: String::new(),
            space_id: None,
            user_id: None,
            purpose: "login".into(),
            return_path: "/".into(),
            invite_hash: None,
            invite_sealed: None,
            google_sub: "s".into(),
            google_email: "a@b.cz".into(),
            google_name: None,
            browser_cookie: String::new(),
            browser_hash: String::new(),
            created_at: Utc::now().into(),
            expires_at: Utc::now().into(),
        };
        assert_eq!(
            identity(&h),
            Identity {
                sub: "s".into(),
                email: "a@b.cz".into(),
                name: None
            }
        );
    }

    #[test]
    fn purposes_and_tokens() {
        for p in [Purpose::Login, Purpose::Link, Purpose::Invite] {
            assert_eq!(Purpose::parse(p.as_str()), Some(p));
        }
        assert_eq!(Purpose::parse("register"), None);
        assert_eq!(url_safe_token("abc-_09"), Some("abc-_09"));
        assert_eq!(url_safe_token(""), None);
        assert_eq!(url_safe_token("a&b=c"), None);
    }

    #[test]
    fn sealed_invite_is_bound_to_the_space() {
        let key = SecretKey::from_bytes([7; 32]);
        let space = SpaceId(Uuid::new_v4());
        let sealed = seal_invite(&key, Some(space), "tok").expect("seal");
        assert_eq!(
            open_invite(&key, Some(space.uuid()), Some(&sealed)).as_deref(),
            Some("tok")
        );
        assert_eq!(open_invite(&key, Some(Uuid::new_v4()), Some(&sealed)), None);
        assert_eq!(open_invite(&key, None, Some(&sealed)), None);
    }
}
