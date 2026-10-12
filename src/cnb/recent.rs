//! A short in-memory cache of the lists that must not go into the
//! `exchange_rates` table (today or later, answered with an older list
//! before ČNB publishes): without it every such lookup refetches.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::NaiveDate;

use super::client::RateList;

/// How long a provisional list is reused: ČNB publishes once a day, so a
/// rate at most this old is as good as a fresh fetch.
pub const TTL: Duration = Duration::from_secs(10 * 60);

/// Provisional lists by requested date.
#[derive(Clone, Default)]
pub struct Recent(Arc<Mutex<HashMap<NaiveDate, (Instant, RateList)>>>);

impl Recent {
    pub fn get(&self, date: NaiveDate) -> Option<RateList> {
        self.get_at(date, Instant::now())
    }

    pub fn put(&self, date: NaiveDate, list: RateList) {
        self.put_at(date, list, Instant::now());
    }

    fn get_at(&self, date: NaiveDate, now: Instant) -> Option<RateList> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&date)
            .filter(|(at, _)| now.duration_since(*at) < TTL)
            .map(|(_, list)| list.clone())
    }

    /// Store `list`, dropping expired entries (a handful of dates at most).
    fn put_at(&self, date: NaiveDate, list: RateList, now: Instant) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (at, _)| now.duration_since(*at) < TTL);
        map.insert(date, (now, list));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("date")
    }

    #[test]
    fn reused_until_the_ttl_passes() {
        let r = Recent::default();
        let list = RateList {
            published: date(1),
            rates: vec![],
        };
        let t0 = Instant::now();
        r.put_at(date(2), list.clone(), t0);
        assert_eq!(r.get_at(date(2), t0 + TTL / 2), Some(list.clone()));
        assert_eq!(r.get_at(date(3), t0), None, "per requested date");
        assert_eq!(r.get_at(date(2), t0 + TTL), None, "expired");
        r.put_at(date(3), list, t0 + TTL);
        let len = r.0.lock().map(|m| m.len()).unwrap_or_default();
        assert_eq!(len, 1, "the expired entry was dropped");
    }
}
