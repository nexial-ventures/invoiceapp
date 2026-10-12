use super::*;

const LIM: Limit = hard("t", 2, Duration::from_secs(60));
const SLOW: Limit = Limit {
    name: "slow",
    max: 3,
    window: Duration::from_secs(900),
    backoff: Some(Backoff {
        base: Duration::from_secs(1),
        cap: Duration::from_secs(30),
    }),
};

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn retry(r: Result<(), AppError>) -> u64 {
    match r {
        Err(AppError::RateLimited(s)) => s,
        other => panic!("expected 429, got {other:?}"),
    }
}

impl RateLimiter {
    fn len(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .len()
    }
}

#[test]
fn sliding_window_and_refund() {
    let rl = RateLimiter::default();
    let t0 = Instant::now();
    assert!(rl.reserve_at(&[(LIM, "k")], t0).is_ok());
    assert!(rl.reserve_at(&[(LIM, "k")], t0 + secs(10)).is_ok());
    assert_eq!(retry(rl.reserve_at(&[(LIM, "k")], t0 + secs(20))), 40);
    assert!(rl.reserve_at(&[(LIM, "other")], t0).is_ok());
    rl.refund(&[(LIM, "k")]);
    assert!(rl.reserve_at(&[(LIM, "k")], t0 + secs(20)).is_ok());
    assert!(rl.reserve_at(&[(LIM, "k")], t0 + secs(70)).is_ok());
}

#[test]
fn retry_after_rounds_up() {
    assert_eq!(retry_after(Duration::from_millis(1)), 1);
    assert_eq!(retry_after(Duration::from_millis(1500)), 2);
    assert_eq!(retry_after(secs(40)), 40);
    assert_eq!(retry_after(Duration::ZERO), 1);
}

#[test]
fn progressive_delay_doubles_after_the_free_attempts() {
    let rl = RateLimiter::default();
    let b = [(SLOW, "k")];
    let mut t = Instant::now();
    for _ in 0..3 {
        rl.reserve_at(&b, t).expect("free attempt");
    }
    // 1 s, 2 s, 4 s, … since the last failure.
    for expected in [1, 2, 4, 8, 16] {
        assert_eq!(retry(rl.reserve_at(&b, t)), expected);
        let just_before = t + secs(expected) - Duration::from_millis(1);
        assert_eq!(retry(rl.reserve_at(&b, just_before)), 1);
        t += secs(expected);
        rl.reserve_at(&b, t).expect("after the wait");
    }
}

#[test]
fn progressive_delay_is_capped() {
    let rl = RateLimiter::default();
    let b = [(SLOW, "k")];
    let mut t = Instant::now();
    for _ in 0..3 {
        rl.reserve_at(&b, t).expect("free attempt");
    }
    for _ in 0..20 {
        let wait = retry(rl.reserve_at(&b, t));
        assert!(wait <= 30, "{wait}");
        t += secs(wait);
        rl.reserve_at(&b, t).expect("after the wait");
    }
    assert_eq!(retry(rl.reserve_at(&b, t)), 30);
}

#[test]
fn success_after_the_wait_is_refunded_and_others_are_unaffected() {
    let rl = RateLimiter::default();
    let t0 = Instant::now();
    let b = [(SLOW, "victim")];
    for _ in 0..3 {
        rl.reserve_at(&b, t0).expect("free failure");
    }
    rl.reserve_at(&b, t0 + secs(1))
        .expect("4th failure, after 1 s");
    assert!(rl.reserve_at(&[(SLOW, "someone")], t0 + secs(1)).is_ok());
    // The correct password after the 2 s wait: reserved, then given back.
    rl.reserve_at(&b, t0 + secs(3)).expect("after the wait");
    rl.refund(&b);
    // The earlier failures stay: still 2 s since the newest failure.
    assert_eq!(retry(rl.reserve_at(&b, t0 + secs(2))), 1);
}

#[test]
fn hard_and_progressive_buckets_together() {
    let rl = RateLimiter::default();
    let t0 = Instant::now();
    for i in 0..2 {
        rl.reserve_at(&[(SLOW, &format!("e{i}")), (LIM, "ip")], t0)
            .expect("free");
    }
    // The hard IP limit refuses regardless of the e-mail's state …
    assert_eq!(retry(rl.reserve_at(&[(SLOW, "e9"), (LIM, "ip")], t0)), 60);
    // … and charges nothing in the refused reservation.
    for _ in 0..3 {
        rl.reserve_at(&[(SLOW, "e9")], t0).expect("e9 untouched");
    }
}

#[test]
fn all_or_nothing() {
    let rl = RateLimiter::default();
    let t0 = Instant::now();
    rl.reserve_at(&[(LIM, "a")], t0).expect("1");
    rl.reserve_at(&[(LIM, "a")], t0).expect("2");
    assert!(rl.reserve_at(&[(LIM, "b"), (LIM, "a")], t0).is_err());
    // "b" was not charged by the refused reservation.
    rl.reserve_at(&[(LIM, "b")], t0).expect("b 1");
    rl.reserve_at(&[(LIM, "b")], t0).expect("b 2");
}

#[test]
fn idle_keys_are_swept() {
    let rl = RateLimiter::default();
    let t0 = Instant::now();
    for i in 0..10 {
        rl.reserve_at(&[(LIM, &format!("k{i}"))], t0).expect("ok");
    }
    rl.reserve_at(&[(SLOW, "active")], t0).expect("ok");
    assert_eq!(rl.len(), 11);
    // LIM keys are idle after their 60 s window; SLOW's lasts 15 min.
    let later = t0 + secs(60);
    for _ in 0..4 {
        rl.reserve_at(&[(LIM, "fresh")], later).ok();
    }
    assert_eq!(rl.len(), 2, "only the active and the fresh key remain");
}

#[test]
fn the_cap_drops_idle_keys_before_active_ones() {
    let rl = RateLimiter::with_cap(8);
    let t0 = Instant::now();
    // A key under attack, inserted first (the oldest): three failures.
    for _ in 0..3 {
        rl.reserve_at(&[(SLOW, "victim")], t0).expect("failure");
    }
    // Pressure: five new keys a second, each idle a second later — more
    // than the sweep removes, so the cap has to act.
    let short = hard("short", 1, secs(1));
    for i in 0..200u64 {
        for j in 0..5 {
            rl.reserve_at(&[(short, &format!("{i}-{j}"))], t0 + secs(i))
                .expect("fresh key");
            assert!(rl.len() <= 8);
        }
    }
    // Still tracked: the fourth attempt passes (its 1 s wait is over), the
    // fifth waits 2 s — a flushed key would pass again.
    let t = t0 + secs(200);
    rl.reserve_at(&[(SLOW, "victim")], t)
        .expect("after the wait");
    assert_eq!(retry(rl.reserve_at(&[(SLOW, "victim")], t)), 2);
}

#[test]
fn over_the_cap_idle_keys_go_first() {
    let rl = RateLimiter::default();
    let t0 = Instant::now();
    rl.reserve_at(&[(SLOW, "victim")], t0).expect("ok");
    for i in 0..10 {
        rl.reserve_at(&[(LIM, &i.to_string())], t0).expect("ok");
    }
    let mut inner = rl.0.lock().expect("lock");
    inner.max_keys = 2;
    inner.cap(t0 + secs(60));
    assert_eq!(
        inner.entries.len(),
        1,
        "every idle key dropped, none active"
    );
    assert!(inner.entries.contains_key(&Inner::key(SLOW, "victim")));
    assert_eq!(inner.order.len(), 1);
    assert!(!inner.warned, "no last-resort eviction");
}

#[test]
fn the_cap_evicts_the_oldest_active_only_as_a_last_resort() {
    let rl = RateLimiter::with_cap(3);
    let t0 = Instant::now();
    for k in ["a", "b", "c", "d"] {
        rl.reserve_at(&[(LIM, k)], t0).expect("ok");
    }
    // All active: one of the older keys had to go, never the new one.
    assert_eq!(rl.len(), 3);
    let inner = rl.0.lock().expect("lock");
    assert!(inner.entries.contains_key(&Inner::key(LIM, "d")));
}

#[test]
fn the_default_cap_bounds_the_map() {
    let rl = RateLimiter::default();
    let t0 = Instant::now();
    for i in 0..MAX_KEYS + 5 {
        rl.reserve_at(&[(LIM, &i.to_string())], t0).expect("ok");
    }
    assert!(rl.len() <= MAX_KEYS);
}

#[test]
fn busy_gives_the_reservation_back() {
    let rl = RateLimiter::default();
    let t0 = Instant::now();
    let b = [(LIM, "k")];
    rl.reserve_at(&b, t0).expect("1");
    assert!(matches!(
        rl.refund_if_busy(&b, Err::<(), _>(AppError::Busy)),
        Err(AppError::Busy)
    ));
    // The busy attempt was given back: two more fit the limit of 2.
    rl.reserve_at(&b, t0).expect("2");
    rl.reserve_at(&b, t0).expect("3");
    rl.refund(&b);
    assert!(
        rl.refund_if_busy(&b, Err::<(), _>(AppError::NotFound))
            .is_err()
    );
    rl.reserve_at(&b, t0).expect("4");
    assert!(
        rl.reserve_at(&b, t0).is_err(),
        "other errors keep the reservation"
    );
    assert_eq!(rl.refund_if_busy(&b, Ok(5)).expect("ok"), 5);
}

#[test]
fn long_keys_are_cut_on_a_char_boundary() {
    let k = Inner::key(LIM, &"ž".repeat(400));
    assert!(k.1.len() <= MAX_KEY);
}

#[test]
fn the_login_limits_are_as_documented() {
    let b = LOGIN_EMAIL.backoff.expect("progressive");
    assert_eq!((LOGIN_EMAIL.max, b.base, b.cap), (3, secs(1), secs(30)));
    assert!(LOGIN_IP.backoff.is_none() && MFA_EMAIL.backoff.is_none());
    assert_eq!((LOGIN_IP.max, MFA_EMAIL.max), (20, 5));
}
