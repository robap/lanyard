//! The in-memory stores: pending authorization requests, authorization codes,
//! sessions, refresh tokens, and the set of revoked token ids.
//!
//! All of them die with the process, on purpose (spec, open question 9).
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

/// **Eight hours, and the only lifetime in this project not measured in
/// seconds.** A working day, because a refresh path you cannot exercise across
/// a lunch break is a refresh path nobody exercises. CONCEPT §6's warning is
/// about *access* tokens being long-lived so the refresh never runs; this is
/// the opposite lever, and the process is the real bound anyway.
pub const REFRESH_TTL: Duration = Duration::from_secs(8 * 60 * 60);

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

    /// Forget an entry **without** leaving a tombstone, and say whether there
    /// was one.
    ///
    /// [`Expiring::take`] is the wrong tool for revocation: its tombstone says
    /// "already exchanged", and a revoked refresh token has a better answer
    /// than that. The caller records the id in [`Revoked`] instead, which is
    /// consulted first and therefore wins.
    pub fn remove(&mut self, id: &str) -> Option<T> {
        self.entries.remove(id).map(|entry| entry.value)
    }

    /// Drop every entry whose value fails `keep`, and hand back the ids that
    /// were dropped.
    ///
    /// The ids come back because logging out has to say *revoked* rather than
    /// *unknown* about the refresh tokens it just dropped, and only the caller
    /// knows where that is recorded. Keyed on the value rather than on a list
    /// of ids the session remembered, because rotation makes such a list stale
    /// the moment it is written.
    pub fn retain(&mut self, keep: impl Fn(&T) -> bool) -> Vec<String> {
        let mut dropped = Vec::new();
        self.entries.retain(|id, entry| {
            if keep(&entry.value) {
                return true;
            }
            dropped.push(id.clone());
            false
        });
        dropped
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

/// Everything a refresh grant needs in order to reach `issue()` with the same
/// arguments the code grant did (north star 3).
///
/// The **whole persona** is owned, not an id, for the same reason
/// [`crate::oidc::code::CodeRecord`] owns one: an identity minted from the
/// picker's "mint one now" panel is in no file and has to travel the identical
/// path. The `nonce` is the *original* one, because OIDC Core §12.2 requires a
/// refreshed ID token to carry it.
#[derive(Debug, Clone)]
pub struct RefreshRecord {
    pub persona: crate::persona::Persona,
    pub client_id: String,
    /// The granted set. A refresh may narrow it and may not widen it.
    pub scope: crate::oidc::scope::Scopes,
    /// The raw string, echoed as the token response's `scope`.
    pub scope_raw: Option<String>,
    /// Becomes the new access token's `aud`, exactly as on the original grant.
    pub audience: Option<String>,
    /// The moment the human actually authenticated, carried forward unchanged.
    pub auth_time: u64,
    pub nonce: Option<String>,
    /// The browser session this grant belongs to, so **Log out** can revoke
    /// what that session holds. Absent for a grant that never had one.
    pub session_id: Option<String>,
}

/// Token ids that have been revoked, and the moment each stops being worth
/// remembering.
///
/// **Access tokens stay stateless JWTs** — Phase 2's contract is untouched — so
/// revoking one can only mean remembering its `jti` until the token would have
/// expired anyway. That bounds the set by the number of unexpired tokens, which
/// for a single-developer tool is a number you can count.
///
/// Refresh token ids land here too, and that is not a second purpose: it is
/// what lets a revoked refresh token be told apart from an unknown one, which
/// [`Expiring`]'s `Spent` tombstone cannot do because it means something else.
///
/// Keyed by the **caller**, unlike [`Expiring`], which chooses its own ids: a
/// `jti` arrives inside a token that already exists.
#[derive(Default)]
pub struct Revoked {
    entries: HashMap<String, Instant>,
}

impl Revoked {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember `id` until `deadline` — the moment the token it names would
    /// have expired anyway. A deadline already past records nothing: there is
    /// nothing left to revoke.
    pub fn revoke(&mut self, id: &str, deadline: Instant) {
        let now = Instant::now();
        self.entries.retain(|_, forget_at| *forget_at > now);
        if deadline > now {
            self.entries.insert(id.to_string(), deadline);
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.entries
            .get(id)
            .is_some_and(|forget_at| *forget_at > Instant::now())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
    /// The access tokens this browser was issued, so **Expire now** can revoke
    /// one `client_id`'s. Refresh tokens are *not* listed here: rotation makes
    /// a recorded id stale immediately, so they are found by scanning the
    /// refresh store for this `session_id` instead.
    issued: Vec<Issuance>,
}

/// One access token this session was issued: which application asked for it,
/// which token it was, and when it dies on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issuance {
    pub client_id: String,
    pub jti: String,
    /// Unix seconds — the token's own `exp`, so a revocation can be forgotten
    /// at exactly the moment it stops mattering.
    pub exp: u64,
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

    /// Everything this browser is signed in as, for the **This browser** panel.
    /// Sorted by `client_id` so the page does not reorder itself on reload.
    pub fn selections(&self, session_id: &str) -> Vec<(String, Selection)> {
        let Some(session) = self.sessions.get(session_id) else {
            return Vec::new();
        };
        let mut rows: Vec<(String, Selection)> = session
            .selections
            .iter()
            .map(|(client_id, selection)| (client_id.clone(), selection.clone()))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// **Log out of lanyard**, and `/oidc/end_session`: the whole record goes.
    pub fn remove(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    /// **Forget**: one `client_id`'s selection, leaving the others alone. The
    /// per-`client_id` variant lives here, on lanyard's own UI where a
    /// developer reaches for it deliberately, and not on `/end_session` where
    /// an SDK would reach it by accident.
    pub fn forget(&mut self, session_id: &str, client_id: &str) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.selections.remove(client_id);
        }
    }

    /// Note an access token this session was issued. Prunes what has expired on
    /// the way in, so the list is bounded by the tokens that are still alive.
    pub fn record_issuance(&mut self, session_id: &str, issuance: Issuance, now: u64) {
        let session = self.sessions.entry(session_id.to_string()).or_default();
        session.issued.retain(|recorded| recorded.exp > now);
        session.issued.push(issuance);
    }

    /// The unexpired access tokens this session holds for one `client_id` —
    /// what **Expire now** revokes.
    pub fn issuances_for(&self, session_id: &str, client_id: &str, now: u64) -> Vec<Issuance> {
        let Some(session) = self.sessions.get(session_id) else {
            return Vec::new();
        };
        session
            .issued
            .iter()
            .filter(|issuance| issuance.client_id == client_id && issuance.exp > now)
            .cloned()
            .collect()
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

/// The stores, hung on [`crate::app::AppState`] behind their own locks.
///
/// A lock each rather than one between them because they are touched at
/// different moments — and because the narrower the guard, the more obvious it
/// is that none of them crosses an `.await`. Two are taken together in exactly
/// one place, logging out, and in a fixed order: refresh, then revoked.
pub struct Stores {
    pub pending: std::sync::Mutex<Expiring<crate::oidc::authorize::AuthRequest>>,
    pub codes: std::sync::Mutex<Expiring<crate::oidc::code::CodeRecord>>,
    pub sessions: std::sync::Mutex<Sessions>,
    pub refresh: std::sync::Mutex<Expiring<RefreshRecord>>,
    pub revoked: std::sync::Mutex<Revoked>,
}

impl Stores {
    /// Shorter lifetimes, so a test can prove the expired-code path without
    /// sleeping for 70 seconds. The acceptance run uses the real 60 and the
    /// real wall clock; this is what keeps the *description* covered in between.
    pub fn with_ttls(pending: Duration, codes: Duration, refresh: Duration) -> Self {
        Stores {
            pending: std::sync::Mutex::new(Expiring::new(pending)),
            codes: std::sync::Mutex::new(Expiring::new(codes)),
            sessions: std::sync::Mutex::new(Sessions::new()),
            refresh: std::sync::Mutex::new(Expiring::new(refresh)),
            revoked: std::sync::Mutex::new(Revoked::new()),
        }
    }
}

impl Default for Stores {
    fn default() -> Self {
        Stores::with_ttls(PENDING_TTL, CODE_TTL, REFRESH_TTL)
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

    /// Mark an entry as having expired a moment ago, leaving it inside the
    /// grace period — the window a zero TTL cannot express, because a zero TTL
    /// makes the grace period zero too.
    ///
    /// The alternative is `thread::sleep`, and it is the reason this exists:
    /// the sleep has to land *between* `ttl` and `2 * ttl`, and a sleep only
    /// ever overshoots. On a busy machine it overshoots past the far edge, the
    /// entry is swept, and the test fails claiming the store forgot something
    /// it had every right to forget. Stating the age is exact and costs no
    /// wall time.
    fn expire<T>(s: &mut Expiring<T>, id: &str) {
        let entry = s.entries.get_mut(id).expect("no entry under that id");
        let now = Instant::now();
        // Saturating: a machine that booted a moment ago has no earlier instant
        // to name, and "now" already counts as expired (`expires_at <= now`).
        entry.expires_at = now.checked_sub(Duration::from_millis(1)).unwrap_or(now);
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
    ///
    /// The 60-second TTL is not patience, it is headroom — the entry expired a
    /// millisecond ago, so the sweep has to keep it for another sixty seconds,
    /// and no scheduling delay can close that window. This test used to sleep
    /// for 120ms against a 100ms TTL, which left 80ms of margin and duly failed
    /// on a loaded CI runner.
    #[test]
    fn a_recently_expired_entry_survives_a_sweep_so_it_can_say_expired() {
        let mut s = store();
        let id = s.insert("stale");
        expire(&mut s, &id);
        s.insert("fresh");
        assert_eq!(
            s.take(&id),
            Lookup::Expired,
            "the sweep dropped an entry that was still inside its grace period"
        );
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

    // ---------------------------------------------------- refresh tokens --

    fn refresh_store() -> Expiring<RefreshRecord> {
        Expiring::new(REFRESH_TTL)
    }

    fn record(session_id: Option<&str>) -> RefreshRecord {
        RefreshRecord {
            persona: crate::persona::Persona {
                id: "ada".into(),
                name: None,
                email: None,
                roles: vec![],
                attributes: serde_json::Map::new(),
                client: None,
            },
            client_id: "billing-web".into(),
            scope: crate::oidc::scope::Scopes::parse(Some("openid offline_access")),
            scope_raw: Some("openid offline_access".into()),
            audience: None,
            auth_time: 1_700_000_000,
            nonce: None,
            session_id: session_id.map(str::to_string),
        }
    }

    /// Rotation *is* the `Spent` tombstone: the store already tells a replayed
    /// id apart from one nobody issued, which is exactly the two
    /// `invalid_grant` descriptions the refresh grant owes.
    #[test]
    fn a_rotated_refresh_token_reads_as_spent_rather_than_unknown() {
        let mut s = refresh_store();
        let first = s.insert(record(Some("sess")));

        let taken = match s.take(&first) {
            Lookup::Found(record) => record,
            other => panic!("the first exchange should find it: {other:?}"),
        };
        let second = s.insert(taken);
        assert_ne!(first, second, "rotation issues a new id");

        assert!(
            matches!(s.take(&first), Lookup::Spent),
            "a replayed refresh token is told it was already exchanged"
        );
        assert!(matches!(s.take("never-issued"), Lookup::Unknown));
    }

    /// `remove` is not `take`: revocation has a better answer than "already
    /// exchanged", so it must not leave the tombstone that says so.
    #[test]
    fn remove_forgets_without_claiming_the_token_was_exchanged() {
        let mut s = refresh_store();
        let id = s.insert(record(None));
        assert!(s.remove(&id).is_some());
        assert!(
            matches!(s.take(&id), Lookup::Unknown),
            "no tombstone: the Revoked set is what says what happened to it"
        );
        assert!(s.remove(&id).is_none(), "removing twice is not an error");
    }

    /// **Log out** revokes what a session holds, and only what that session
    /// holds. Keyed on the record rather than on a list of ids, because
    /// rotation makes such a list stale the moment it is written.
    #[test]
    fn retain_by_session_id_drops_only_that_sessions_refresh_tokens() {
        let mut s = refresh_store();
        let mine = s.insert(record(Some("mine")));
        let also_mine = s.insert(record(Some("mine")));
        let theirs = s.insert(record(Some("theirs")));
        let sessionless = s.insert(record(None));

        let mut dropped = s.retain(|r| r.session_id.as_deref() != Some("mine"));
        dropped.sort();
        let mut expected = vec![mine.clone(), also_mine.clone()];
        expected.sort();
        assert_eq!(
            dropped, expected,
            "the ids come back so they can be revoked"
        );

        assert!(matches!(s.take(&mine), Lookup::Unknown));
        assert!(matches!(s.take(&also_mine), Lookup::Unknown));
        assert!(matches!(s.take(&theirs), Lookup::Found(_)));
        assert!(matches!(s.take(&sessionless), Lookup::Found(_)));
    }

    // ---------------------------------------------------------- revoked --

    #[test]
    fn a_revoked_id_is_remembered_until_its_deadline_and_then_forgotten() {
        let mut r = Revoked::new();
        r.revoke("jti-1", Instant::now() + Duration::from_secs(60));
        assert!(r.contains("jti-1"));
        assert!(!r.contains("jti-2"), "and nothing else");

        r.revoke("jti-3", Instant::now() + Duration::from_millis(30));
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            !r.contains("jti-3"),
            "past its deadline the token had expired anyway"
        );
    }

    /// Bounded without a background task, like everything else here: the sweep
    /// happens on the way in.
    #[test]
    fn revoking_sweeps_deadlines_that_have_passed() {
        let mut r = Revoked::new();
        for i in 0..10 {
            r.revoke(&format!("dead-{i}"), Instant::now());
        }
        assert_eq!(r.len(), 0, "a deadline already past records nothing");

        r.revoke("live", Instant::now() + Duration::from_secs(60));
        assert_eq!(r.len(), 1);
    }

    // ------------------------------------------- what a session was issued --

    fn issuance(client_id: &str, jti: &str, exp: u64) -> Issuance {
        Issuance {
            client_id: client_id.to_string(),
            jti: jti.to_string(),
            exp,
        }
    }

    /// **Expire now** is per `client_id`: it revokes one application's tokens
    /// and leaves the other application's alone.
    #[test]
    fn issuances_come_back_per_client_id_and_only_while_unexpired() {
        let mut s = Sessions::new();
        let sid = s.ensure(None);
        let now = 1_700_000_000;

        s.record_issuance(&sid, issuance("billing-web", "a", now + 60), now);
        s.record_issuance(&sid, issuance("spike-php", "b", now + 60), now);
        s.record_issuance(&sid, issuance("billing-web", "old", now - 1), now);

        let mine = s.issuances_for(&sid, "billing-web", now);
        assert_eq!(
            mine.iter().map(|i| i.jti.as_str()).collect::<Vec<_>>(),
            ["a"],
            "a token that has already expired needs no revoking"
        );
        assert_eq!(s.issuances_for(&sid, "spike-php", now).len(), 1);
        assert!(s.issuances_for("nobody", "billing-web", now).is_empty());
    }

    /// The list is bounded by what is still alive: recording prunes.
    #[test]
    fn recording_an_issuance_prunes_the_ones_that_have_expired() {
        let mut s = Sessions::new();
        let sid = s.ensure(None);
        let now = 1_700_000_000;
        for i in 0..10 {
            s.record_issuance(
                &sid,
                issuance("billing-web", &format!("j{i}"), now),
                now + 1,
            );
        }
        assert_eq!(
            s.issuances_for(&sid, "billing-web", now + 1).len(),
            0,
            "every one of them was already past its exp"
        );
    }

    /// **Log out of lanyard** drops the whole record — every selection and the
    /// always-ask flag with it.
    #[test]
    fn remove_drops_the_whole_session_and_forget_drops_one_client() {
        let mut s = Sessions::new();
        let sid = s.ensure(None);
        s.remember(&sid, "billing-web", selection("ada"));
        s.remember(&sid, "spike-php", selection("mira"));

        s.forget(&sid, "billing-web");
        assert_eq!(s.selection(&sid, "billing-web"), None);
        assert_eq!(
            s.selection(&sid, "spike-php").unwrap().persona_id,
            "mira",
            "the other rows are untouched"
        );

        s.remove(&sid);
        assert_eq!(s.selection(&sid, "spike-php"), None);
        assert!(s.selections(&sid).is_empty());
        // Logging nobody out is not an error, and neither is forgetting a
        // client that was never chosen.
        s.remove("nobody");
        s.forget("nobody", "billing-web");
    }

    /// The **This browser** panel's rows, in an order the page can rely on.
    #[test]
    fn selections_lists_every_client_in_a_stable_order() {
        let mut s = Sessions::new();
        let sid = s.ensure(None);
        s.remember(&sid, "spike-php", selection("mira"));
        s.remember(&sid, "billing-web", selection("ada"));

        let rows = s.selections(&sid);
        assert_eq!(
            rows.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(),
            ["billing-web", "spike-php"],
            "sorted, so the page does not reorder itself on reload"
        );
        assert_eq!(rows[0].1.persona_id, "ada");
    }
}
