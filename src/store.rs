//! The three in-memory stores the browser flow needs: pending authorization
//! requests, authorization codes, and sessions.
//!
//! All three die with the process, on purpose (spec, open question 9).
//! Restarting `lanyard serve` logs everybody out and clears every half-finished
//! login, which is a property worth having in a tool whose reset story is
//! "restart it".
//!
//! **Nothing here ever awaits.** Every method takes its lock, does its work, and
//! returns; the handlers hold no guard across an `.await`. A `std::sync::Mutex`
//! held across a yield point deadlocks the multi-thread runtime, and the
//! discipline that prevents it is "the store has no async surface at all".

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// An authorization request survives five minutes — long enough for a human to
/// look at the picker and pick.
pub const PENDING_TTL: Duration = Duration::from_secs(300);

/// A code survives 60 seconds. It is issued *after* the human has clicked, so
/// the only thing that has to fit inside it is a redirect and an exchange.
pub const CODE_TTL: Duration = Duration::from_secs(60);

/// What a lookup found. Four answers rather than an `Option`, because
/// `/oidc/token` owes a *different* `error_description` to an unknown code, an
/// expired one, and one that has already been exchanged — criterion 12 asks for
/// exactly those three to be distinguishable, and Phase 6's log can only report
/// what this phase distinguishes.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup<T> {
    Found(T),
    Expired,
    /// Taken once already. This is what single-use *means* to the caller: not
    /// "gone" but "you already had it".
    Spent,
    Unknown,
}

struct Entry<T> {
    expires_at: Instant,
    value: T,
}

/// A map whose entries stop being valid after `ttl`, keyed by an unguessable id.
///
/// Sweeping happens on insert, so there is no background task to own, start,
/// stop or leak.
pub struct Expiring<T> {
    ttl: Duration,
    entries: HashMap<String, Entry<T>>,
    /// Ids that were taken, and when they stop being worth remembering. A
    /// tombstone rather than a silent deletion, so a replayed code can be told
    /// it is a replay.
    spent: HashMap<String, Instant>,
}

impl<T> Expiring<T> {
    pub fn new(ttl: Duration) -> Self {
        Expiring {
            ttl,
            entries: HashMap::new(),
            spent: HashMap::new(),
        }
    }

    /// Store `value` under a fresh id and return it. Sweeps first, so the map is
    /// bounded by what is actually in flight.
    pub fn insert(&mut self, value: T) -> String {
        self.sweep();
        let id = new_id();
        self.entries.insert(
            id.clone(),
            Entry {
                expires_at: Instant::now() + self.ttl,
                value,
            },
        );
        id
    }

    /// Read **and remove**. This is what makes an authorization code single-use:
    /// a second exchange of the same code finds nothing, and gets a different
    /// message than an unknown code would.
    pub fn take(&mut self, id: &str) -> Lookup<T> {
        let now = Instant::now();
        match self.entries.remove(id) {
            None if self.spent.contains_key(id) => Lookup::Spent,
            None => Lookup::Unknown,
            Some(entry) if entry.expires_at <= now => Lookup::Expired,
            Some(entry) => {
                self.spent.insert(id.to_string(), now + self.ttl + self.ttl);
                Lookup::Found(entry.value)
            }
        }
    }

    /// Read without removing — the picker renders the same pending request as
    /// many times as it is reloaded, and only `/_/pick` consumes it.
    pub fn peek(&self, id: &str) -> Lookup<&T> {
        match self.entries.get(id) {
            None if self.spent.contains_key(id) => Lookup::Spent,
            None => Lookup::Unknown,
            Some(entry) if entry.expires_at <= Instant::now() => Lookup::Expired,
            Some(entry) => Lookup::Found(&entry.value),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drops entries that expired **at least another `ttl` ago**, not merely
    /// entries that expired.
    ///
    /// The grace period is the whole point: a code presented 70 seconds after
    /// issue has to be able to say *"expired"* rather than *"unknown"*, and it
    /// can only say that if its record is still here to be found. Memory stays
    /// bounded at twice the lifetime of what is in flight, which for a
    /// single-developer tool is a number of records you can count on one hand.
    fn sweep(&mut self) {
        let now = Instant::now();
        self.entries
            .retain(|_, entry| entry.expires_at + self.ttl > now);
        self.spent.retain(|_, forget_at| *forget_at > now);
    }
}

/// What one `client_id` in one browser is currently logged in as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub persona_id: String,
    /// Unix seconds, because it becomes the ID token's `auth_time` — which may
    /// legitimately predate `iat` on a remembered session.
    pub auth_time: u64,
}

/// One browser profile's record: what it is logged in as **per `client_id`**,
/// plus one global "always ask" flag.
///
/// Per-`client_id` is the whole multi-project property (CONCEPT §3). One
/// instance serving three apps on three ports must not let a persona chosen in
/// one leak into another, and this map is where that is true or false.
#[derive(Debug, Default)]
pub struct Session {
    selections: HashMap<String, Selection>,
    pub always_ask: bool,
}

/// Sessions have **no expiry**: the cookie carrying the id has no `Max-Age`, so
/// closing the browser is the reset, and the process is the outer bound.
#[derive(Default)]
pub struct Sessions {
    sessions: HashMap<String, Session>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The id from the cookie if it names a session we know, otherwise a fresh
    /// one. A cookie from a previous run of the process names nothing and is
    /// silently replaced — that is criterion 22 working, not an error.
    pub fn ensure(&mut self, existing: Option<&str>) -> String {
        if let Some(id) = existing {
            if self.sessions.contains_key(id) {
                return id.to_string();
            }
        }
        let id = new_id();
        self.sessions.insert(id.clone(), Session::default());
        id
    }

    pub fn selection(&self, session_id: &str, client_id: &str) -> Option<&Selection> {
        self.sessions.get(session_id)?.selections.get(client_id)
    }

    pub fn remember(&mut self, session_id: &str, client_id: &str, selection: Selection) {
        self.sessions
            .entry(session_id.to_string())
            .or_default()
            .selections
            .insert(client_id.to_string(), selection);
    }

    pub fn always_ask(&self, session_id: &str) -> bool {
        self.sessions.get(session_id).is_some_and(|s| s.always_ask)
    }

    pub fn set_always_ask(&mut self, session_id: &str, value: bool) {
        self.sessions
            .entry(session_id.to_string())
            .or_default()
            .always_ask = value;
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

/// Unguessable, url-safe, and short enough to read in a browser address bar.
/// A v4 UUID's 122 random bits is the same unguessability the authorization code
/// itself needs, so both use it.
fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The three stores, hung on [`crate::app::AppState`] behind their own locks.
///
/// Three locks rather than one because they are touched at different moments
/// and nothing ever needs two at once — and because the narrower the guard, the
/// more obvious it is that none of them crosses an `.await`.
pub struct Stores {
    pub pending: std::sync::Mutex<Expiring<crate::oidc::authorize::AuthRequest>>,
    pub codes: std::sync::Mutex<Expiring<crate::oidc::code::CodeRecord>>,
    pub sessions: std::sync::Mutex<Sessions>,
}

impl Stores {
    /// Shorter lifetimes, so a test can prove the expired-code path without
    /// sleeping for 70 seconds. The acceptance run uses the real 60 and the
    /// real wall clock; this is what keeps the *description* covered in between.
    pub fn with_ttls(pending: Duration, codes: Duration) -> Self {
        Stores {
            pending: std::sync::Mutex::new(Expiring::new(pending)),
            codes: std::sync::Mutex::new(Expiring::new(codes)),
            sessions: std::sync::Mutex::new(Sessions::new()),
        }
    }
}

impl Default for Stores {
    fn default() -> Self {
        Stores::with_ttls(PENDING_TTL, CODE_TTL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Expiring<&'static str> {
        Expiring::new(Duration::from_secs(60))
    }

    /// Zero TTL: an entry that is already expired the moment it lands, so expiry
    /// is testable without sleeping and without a clock injection nobody else
    /// needs.
    fn dead_store() -> Expiring<&'static str> {
        Expiring::new(Duration::ZERO)
    }

    #[test]
    fn an_inserted_value_comes_back_under_its_id() {
        let mut s = store();
        let id = s.insert("ada");
        assert_eq!(s.peek(&id), Lookup::Found(&"ada"));
        assert_eq!(s.take(&id), Lookup::Found("ada"));
    }

    #[test]
    fn ids_are_unguessable_and_never_repeat() {
        let mut s = store();
        let a = s.insert("ada");
        let b = s.insert("mira");
        assert_ne!(a, b);
        assert!(a.len() >= 32, "a 32-hex v4 uuid, not a counter: {a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "{a}");
    }

    /// The property that makes a code single-use. A second exchange finds
    /// nothing.
    #[test]
    fn take_removes_so_a_second_take_finds_nothing() {
        let mut s = store();
        let id = s.insert("code");
        assert_eq!(s.take(&id), Lookup::Found("code"));
        assert_eq!(
            s.take(&id),
            Lookup::Spent,
            "a replayed code is told it is a replay, not that it never existed"
        );
    }

    #[test]
    fn peek_does_not_remove_so_the_picker_can_be_reloaded() {
        let mut s = store();
        let id = s.insert("req");
        assert_eq!(s.peek(&id), Lookup::Found(&"req"));
        assert_eq!(s.peek(&id), Lookup::Found(&"req"));
        assert_eq!(s.take(&id), Lookup::Found("req"));
    }

    /// Expired is not Unknown, and the difference is the whole reason `Lookup`
    /// has three variants: `/oidc/token` owes two different descriptions.
    #[test]
    fn an_expired_entry_reads_as_expired_rather_than_unknown() {
        let mut s = dead_store();
        let id = s.insert("stale");
        assert_eq!(s.peek(&id), Lookup::Expired);
        assert_eq!(s.take(&id), Lookup::Expired);
        assert_eq!(
            s.take(&id),
            Lookup::Unknown,
            "taking an expired entry removes it, and it was never spent"
        );
    }

    /// The tombstone outlives the record but not forever: bounded like
    /// everything else here, by a sweep on insert.
    #[test]
    fn a_spent_id_is_eventually_forgotten_rather_than_remembered_for_ever() {
        let mut s = Expiring::new(Duration::ZERO);
        let id = s.insert("code");
        assert_eq!(s.take(&id), Lookup::Expired, "zero ttl expires on arrival");

        let mut s = Expiring::new(Duration::from_millis(50));
        let id = s.insert("code");
        assert_eq!(s.take(&id), Lookup::Found("code"));
        assert_eq!(s.take(&id), Lookup::Spent);
        std::thread::sleep(Duration::from_millis(120));
        s.insert("something else");
        assert_eq!(s.take(&id), Lookup::Unknown);
    }

    #[test]
    fn an_id_nobody_issued_is_unknown() {
        let mut s = store();
        assert_eq!(s.peek("deadbeef"), Lookup::Unknown);
        assert_eq!(s.take("deadbeef"), Lookup::Unknown);
    }

    /// Bounded without a background task: inserting sweeps what is long dead.
    /// With a zero TTL every previous entry is already past its grace period, so
    /// the map never holds more than the one just added.
    #[test]
    fn inserting_sweeps_entries_that_are_long_expired() {
        let mut s = dead_store();
        for _ in 0..10 {
            s.insert("stale");
        }
        assert_eq!(s.len(), 1, "nine sweeps happened, one entry survives");
    }

    /// The grace period, stated as a test: a *recently* expired entry is still
    /// there to be found, so it can report `Expired` rather than `Unknown`.
    #[test]
    fn a_recently_expired_entry_survives_a_sweep_so_it_can_say_expired() {
        let mut s = Expiring::new(Duration::from_millis(100));
        let id = s.insert("stale");
        std::thread::sleep(Duration::from_millis(120));
        s.insert("fresh");
        assert_eq!(s.take(&id), Lookup::Expired);
    }

    // ------------------------------------------------------------ sessions --

    fn selection(id: &str) -> Selection {
        Selection {
            persona_id: id.to_string(),
            auth_time: 1_700_000_000,
        }
    }

    #[test]
    fn an_unknown_cookie_yields_a_fresh_session_rather_than_an_error() {
        let mut s = Sessions::new();
        let fresh = s.ensure(None);
        let recycled = s.ensure(Some(&fresh));
        assert_eq!(fresh, recycled, "a known id is kept");

        // A cookie left over from a previous run of the process names nothing.
        let replaced = s.ensure(Some("from-a-previous-boot"));
        assert_ne!(replaced, "from-a-previous-boot");
    }

    #[test]
    fn a_remembered_selection_comes_back_for_that_client() {
        let mut s = Sessions::new();
        let sid = s.ensure(None);
        s.remember(&sid, "billing-web", selection("ada"));
        assert_eq!(s.selection(&sid, "billing-web"), Some(&selection("ada")));
    }

    /// CONCEPT §3's central claim, as a unit test: two apps in one browser do
    /// not see each other's person.
    #[test]
    fn two_client_ids_in_one_session_do_not_see_each_others_persona() {
        let mut s = Sessions::new();
        let sid = s.ensure(None);
        s.remember(&sid, "billing-web", selection("ada"));
        assert_eq!(s.selection(&sid, "spike-php"), None);

        s.remember(&sid, "spike-php", selection("mira"));
        assert_eq!(s.selection(&sid, "spike-php").unwrap().persona_id, "mira");
        assert_eq!(
            s.selection(&sid, "billing-web").unwrap().persona_id,
            "ada",
            "choosing in one app must not disturb the other"
        );
    }

    #[test]
    fn two_sessions_do_not_see_each_other_either() {
        let mut s = Sessions::new();
        let one = s.ensure(None);
        let two = s.ensure(None);
        s.remember(&one, "billing-web", selection("ada"));
        assert_eq!(s.selection(&two, "billing-web"), None);
    }

    /// One flag for the whole browser, not per client — criterion 21 says
    /// enabling it shows the picker for *both* client ids.
    #[test]
    fn always_ask_is_off_by_default_and_covers_the_whole_session() {
        let mut s = Sessions::new();
        let sid = s.ensure(None);
        assert!(!s.always_ask(&sid));

        s.set_always_ask(&sid, true);
        assert!(s.always_ask(&sid));
        s.set_always_ask(&sid, false);
        assert!(!s.always_ask(&sid), "unchecking restores the skip");
    }

    #[test]
    fn always_ask_on_a_session_nobody_started_is_false_rather_than_a_panic() {
        let s = Sessions::new();
        assert!(!s.always_ask("nothing"));
    }
}
