//! The limits themselves (`docs/api/auth.md`, rate limits).

use std::time::Duration;

const FIFTEEN_MIN: Duration = Duration::from_secs(15 * 60);
const HOUR: Duration = Duration::from_secs(60 * 60);

/// One limit: `max` events per `window` pass freely. Beyond that a **hard**
/// limit (`backoff: None`) refuses until the oldest event leaves the window;
/// a **progressive** one only makes each further event wait since the newest
/// (see [`Backoff`]).
#[derive(Debug, Clone, Copy)]
pub struct Limit {
    pub name: &'static str,
    pub max: usize,
    pub window: Duration,
    pub backoff: Option<Backoff>,
}

/// With `n ≥ max` events in the window the next one waits
/// `base · 2^(n − max)`, at most `cap`, since the newest event.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub base: Duration,
    pub cap: Duration,
}

/// Failed logins per e-mail **and client** ([`login_key`]); also failed
/// own-password checks of a signed-in user (password change, space delete,
/// TOTP setup, …). Progressive and per client: the e-mail is anyone's to
/// type, so a guesser only ever delays their own attempts.
pub const LOGIN_EMAIL: Limit = Limit {
    name: "login-email",
    max: 3,
    window: FIFTEEN_MIN,
    backoff: Some(Backoff {
        base: Duration::from_secs(1),
        cap: Duration::from_secs(30),
    }),
};
/// All failures of one e-mail from every client, keyed by the e-mail: the
/// backstop against a guess spread over many networks. One network cannot
/// reach it (the IP limit stops it at 20), so holding an account at 429
/// takes at least five — the accepted residual lockout (`docs/api/auth.md`).
pub const LOGIN_EMAIL_ALL: Limit = hard("login-email-all", 100, FIFTEEN_MIN);
pub const LOGIN_IP: Limit = hard("login-ip", 20, FIFTEEN_MIN);
/// Failed TOTP / recovery codes per user (login step, step-up, setup).
/// Hard: reaching a code check takes the password or a session, so a
/// stranger cannot lock it, and a 6-digit code must not get the far
/// higher rate of the progressive login delay.
pub const MFA_EMAIL: Limit = hard("mfa-email", 5, FIFTEEN_MIN);
/// Register, verify/resend and password-reset share these two buckets.
pub const MAIL_IP: Limit = hard("mail-ip", 5, HOUR);
pub const MAIL_EMAIL: Limit = hard("mail-email", 3, HOUR);
/// Password-reset confirmations per IP (every request counts).
pub const RESET_IP: Limit = hard("reset-ip", 20, FIFTEEN_MIN);
/// Invitation lookups and accepts per IP (every request counts).
pub const INVITE_IP: Limit = hard("invite-ip", 20, FIFTEEN_MIN);
/// Google sign-in starts per IP (every request counts).
pub const GOOGLE_IP: Limit = hard("google-ip", 20, FIFTEEN_MIN);
/// Invitation e-mails (create + resend) per inviting user and per space.
pub const INVITE_USER: Limit = hard("invite-user", 20, HOUR);
pub const INVITE_SPACE: Limit = hard("invite-space", 50, HOUR);
/// Design activations and restores per space (each costs two renders).
pub const DESIGN_SPACE: Limit = hard("design-space", 10, HOUR);

/// The [`LOGIN_EMAIL`] key: one e-mail from one client (its rate-limit IP
/// key — an IPv6 client by its /64).
pub fn login_key(email: &str, ip: &str) -> String {
    format!("{email} from {ip}")
}

pub const fn hard(name: &'static str, max: usize, window: Duration) -> Limit {
    Limit {
        name,
        max,
        window,
        backoff: None,
    }
}

impl Limit {
    /// How long a key stays relevant after its newest event.
    pub(super) fn span(&self) -> Duration {
        match self.backoff {
            Some(b) => self.window.max(b.cap),
            None => self.window,
        }
    }

    /// The wait before one more event, given the (pruned) events in the
    /// window, oldest first; `None` = it may happen now.
    pub(super) fn wait(
        &self,
        events: &std::collections::VecDeque<std::time::Instant>,
        now: std::time::Instant,
    ) -> Option<Duration> {
        let n = events.len();
        if n < self.max {
            return None;
        }
        match self.backoff {
            None => {
                let oldest = events.front()?;
                Some((*oldest + self.window).saturating_duration_since(now))
            }
            Some(b) => {
                let newest = events.back()?;
                let doublings = u32::try_from(n - self.max).unwrap_or(u32::MAX).min(16);
                let delay = b.base.saturating_mul(1 << doublings).min(b.cap);
                let wait = (*newest + delay).saturating_duration_since(now);
                (!wait.is_zero()).then_some(wait)
            }
        }
    }
}
