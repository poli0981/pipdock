//! Process-wide state the commands share.
//!
//! Two things force this to exist rather than opening what each command needs:
//!
//! * `Store` wraps a `rusqlite::Connection`, which is `Send` but **not `Sync`**, so
//!   `tauri::State<Store>` does not compile. It needs the mutex.
//! * `NameIndex` **must** be held across calls. Spike SP-3 measured scanning SQLite at 218 ms per
//!   keystroke against a 50 ms budget; loading it per `index_search` would reintroduce exactly the
//!   failure that spike ruled out.
//!
//! A third arrived with S3, and it is the reason the mutation spine works at all: `UpdateFlow` is
//! a **resumable state machine** and IPC is stateless. `plan_resolve` builds the flow, the user
//! looks at the preview, and `plan_decide` or `plan_execute` arrives as a separate message some
//! seconds later. Something has to hold the flow in between, and that is [`Sessions`].
//!
//! The slot and its cancellation token live in [`Sessions`] rather than directly on [`AppState`]
//! because their protocol is the delicate part — claim, park, release, exactly once each — and it
//! is worth testing on its own. [`Sessions`] is generic over what it holds for the same reason: a
//! test can drive the protocol with a `u32` where it could never construct an `UpdateFlow`, which
//! needs a real interpreter to probe.

use std::path::PathBuf;

use pipdock_core::errors::{Code, PdError, Result};
use pipdock_core::flow::{RollbackFlow, UninstallFlow, UpdateFlow};
use pipdock_core::index::NameIndex;
use pipdock_core::store::Store;
use tokio_util::sync::CancellationToken;

/// Which mutation a parked session is part-way through.
///
/// Separate from [`Session`] so a mismatch can be *reported* — "expected an update, found an
/// uninstall" names a real bug in the caller, where a bare "no plan in progress" would send
/// whoever reads it looking in the wrong place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// An update or an install: `UpdateFlow`, the only one with a decision loop.
    Update,
    /// A removal: `UninstallFlow`.
    Uninstall,
    /// A restore: `RollbackFlow`.
    Rollback,
}

impl SessionKind {
    /// The wire name, for error messages. Never localized (I18N §2).
    const fn as_str(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Uninstall => "uninstall",
            Self::Rollback => "rollback",
        }
    }
}

/// A finished Code Health run, parked so the fix path can be checked against it.
///
/// The report is held **server-side** on purpose. P5's `health_fix` must confirm that what it is
/// about to rewrite is what the user was shown, and a count that arrives from the frontend proves
/// nothing about what the frontend displayed. Same reasoning as `GuardReport`: the artefact is
/// produced in one call and consumed in another, so the consuming call reads it from here.
pub struct HealthSession {
    /// The folder the tools ran in.
    pub project: std::path::PathBuf,
    /// The environment the report was produced against.
    pub env: pipdock_core::PyEnv,
    /// What they found.
    pub report: pipdock_core::health::HealthReport,
}

/// The mutation flow the slot is holding between two IPC calls.
///
/// All three variants exist from the moment the slot stops being `UpdateFlow`-shaped, rather than
/// arriving one per slice. The alternative — widen it again for S6 — means revisiting `claim`,
/// `park` and every match a second time, and the enum is the cheap half of the work.
///
/// Rejected: `Box<dyn MutationFlow>`. The three flows do not share a signature — only `UpdateFlow`
/// decides, only `UninstallFlow` has `names`, only `RollbackFlow` produces a preview — so the
/// trait would be a pile of methods returning `Option`, and every caller would still be matching
/// on which one it really had.
pub enum Session {
    /// `plan_resolve` → `plan_decide`* → `plan_execute`.
    Update(Box<UpdateFlow>),
    /// `uninstall_guard` → `uninstall_execute`.
    Uninstall(Box<UninstallFlow>),
    /// `snapshot_rollback_preview` → `snapshot_rollback`.
    Rollback(Box<RollbackFlow>),
    /// Test-only, so the slot protocol can be exercised without an interpreter to probe.
    #[cfg(test)]
    Fake(SessionKind),
}

impl Session {
    /// Which flow this is.
    pub const fn kind(&self) -> SessionKind {
        match *self {
            Self::Update(_) => SessionKind::Update,
            Self::Uninstall(_) => SessionKind::Uninstall,
            Self::Rollback(_) => SessionKind::Rollback,
            #[cfg(test)]
            Self::Fake(kind) => kind,
        }
    }
}

/// Where the one plan this session may have has got to.
///
/// One at a time, deliberately. Two concurrent plans would interleave engine commands against a
/// single environment, and DATA-FLOW §9.3's staleness check would be comparing the installed set
/// against one the other plan is busy changing — so the second is refused with `PD-RES-003`
/// rather than allowed to race.
pub enum PlanSlot<T> {
    /// No plan.
    Idle,
    /// A flow is waiting for the user: a decision, or the final confirm.
    Ready(T),
    /// Engine work is in flight, so the flow is owned by whichever command is driving it.
    ///
    /// The flow is *moved out* for the duration rather than borrowed, because `execute` awaits for
    /// as long as the install takes and holding the slot's lock across that would block
    /// `plan_cancel` — the one command that must work precisely then.
    Busy,
}

impl<T> PlanSlot<T> {
    /// True while engine work is in flight.
    pub const fn is_busy(&self) -> bool {
        matches!(*self, Self::Busy)
    }
}

/// The one mutation session this process may have, and the token that stops it.
///
/// Generic over the parked value so the protocol below can be tested without an `UpdateFlow`.
pub struct Sessions<T> {
    /// A `std` mutex, not tokio's: it is only ever held to read or replace the slot, never across
    /// an await, and [`Claim`]'s `Drop` has to be able to take it — a drop cannot await.
    slot: std::sync::Mutex<PlanSlot<T>>,
    /// The token for whatever engine work is in flight.
    ///
    /// Deliberately **not** inside [`PlanSlot`]: `plan_cancel` must be able to trip it while the
    /// slot is `Busy` and its flow is owned elsewhere. Its own lock is only ever held for the
    /// moment it takes to read or replace an `Option`, never across an await.
    cancel: std::sync::Mutex<Option<CancellationToken>>,
}

impl<T> Default for Sessions<T> {
    fn default() -> Self {
        Self {
            slot: std::sync::Mutex::new(PlanSlot::Idle),
            cancel: std::sync::Mutex::new(None),
        }
    }
}

/// A claimed slot. **Dropping it releases the slot** — on every path out of a command, the `?`s
/// included — unless it was [`Self::park`]ed first.
///
/// Release used to be a call each exit path had to remember, and three did not: `plan_resolve`,
/// `uninstall_guard` and the rollback preview all claimed and then had a `?` before their first
/// release, so a settings read that failed left the slot `Busy` and every later plan answered
/// `PD-RES-003` until the app was restarted (U4). A guard makes forgetting unrepresentable.
#[must_use = "dropping a claim releases the slot at once"]
pub struct Claim<'a, T> {
    sessions: &'a Sessions<T>,
    settled: bool,
}

impl<T> Claim<'_, T> {
    /// Put a session back for the next call to pick up. Consumes the claim.
    pub fn park(mut self, session: T) {
        self.sessions.put(PlanSlot::Ready(session));
        self.settled = true;
    }
}

impl<T> Drop for Claim<'_, T> {
    fn drop(&mut self) {
        if !self.settled {
            self.sessions.put(PlanSlot::Idle);
            self.sessions.set_cancel(None);
        }
    }
}

impl<T> Sessions<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, PlanSlot<T>> {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn put(&self, next: PlanSlot<T>) {
        *self.lock() = next;
    }

    /// Claim the slot for work that is about to *start*, with whatever was parked.
    ///
    /// The empty case is a successful claim — `plan_resolve` is beginning a plan and must exclude
    /// a second one for the whole of the resolve. A caller that needs an *existing* session wants
    /// [`Self::claim_one`] instead.
    ///
    /// # Errors
    /// `PD-RES-003` when a plan is already in flight.
    pub fn claim(&self) -> Result<(Claim<'_, T>, Option<T>)> {
        let mut slot = self.lock();
        if slot.is_busy() {
            return Err(in_flight());
        }
        let parked = match std::mem::replace(&mut *slot, PlanSlot::Busy) {
            PlanSlot::Ready(session) => Some(session),
            PlanSlot::Idle | PlanSlot::Busy => None,
        };
        drop(slot);
        Ok((
            Claim {
                sessions: self,
                settled: false,
            },
            parked,
        ))
    }

    /// Claim the session a caller expects to already be parked.
    ///
    /// The difference from [`Self::claim`] is the empty case, and it is the whole reason this
    /// method exists: finding nothing is not claiming anything, so the slot is left idle. The old
    /// code wrote `Busy` before discovering there was nothing to take, and one out-of-order call
    /// from the UI wedged the slot for the rest of the process.
    ///
    /// # Errors
    /// `PD-RES-003` when a plan is already in flight; `PD-INT-001` when there is nothing parked.
    pub fn claim_one(&self) -> Result<(Claim<'_, T>, T)> {
        let mut slot = self.lock();
        if slot.is_busy() {
            return Err(in_flight());
        }
        match std::mem::replace(&mut *slot, PlanSlot::Busy) {
            PlanSlot::Ready(session) => {
                drop(slot);
                Ok((
                    Claim {
                        sessions: self,
                        settled: false,
                    },
                    session,
                ))
            }
            PlanSlot::Idle | PlanSlot::Busy => {
                *slot = PlanSlot::Idle;
                drop(slot);
                self.set_cancel(None);
                Err(no_session())
            }
        }
    }

    /// Record the token for the work about to start.
    pub fn set_cancel(&self, token: Option<CancellationToken>) {
        if let Ok(mut guard) = self.cancel.lock() {
            *guard = token;
        }
    }

    /// Trip the in-flight token, if there is one. Returns whether anything was cancelled.
    pub fn cancel_current(&self) -> bool {
        let token: Option<CancellationToken> = self
            .cancel
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned());
        match token {
            Some(t) => {
                t.cancel();
                true
            }
            None => false,
        }
    }

    /// Stop whatever the session is doing, whichever half of it that is.
    ///
    /// Two states both count as "there was something to stop", and only one of them is a running
    /// process. Engine work in flight is cancelled through the token, and the command running it
    /// releases the slot when its claim drops. A session merely *parked* — a preview on screen, a
    /// guard the user is reading — has no process to kill and is discarded instead, because
    /// leaving it there means the next plan is refused by a flow nobody is looking at any more.
    pub fn stop(&self) -> bool {
        let mut slot = self.lock();
        if slot.is_busy() {
            drop(slot);
            return self.cancel_current();
        }
        let was_parked = matches!(*slot, PlanSlot::Ready(_));
        *slot = PlanSlot::Idle;
        drop(slot);
        self.set_cancel(None);
        was_parked
    }

    /// Put a session in the slot without having claimed it. For tests of what comes after.
    #[cfg(test)]
    pub fn park_for_test(&self, session: T) {
        self.put(PlanSlot::Ready(session));
    }
}

/// A claimed slot holding the mutation session.
pub type SessionClaim<'a> = Claim<'a, Session>;

impl Sessions<Session> {
    /// Claim a parked [`Session::Update`].
    ///
    /// # Errors
    /// `PD-RES-003` when a plan is in flight, `PD-INT-001` when nothing is parked or the parked
    /// session is a different flow.
    pub fn claim_update(&self) -> Result<(SessionClaim<'_>, Box<UpdateFlow>)> {
        match self.claim_one()? {
            (claim, Session::Update(flow)) => Ok((claim, flow)),
            (claim, other) => Err(wrong_kind(claim, SessionKind::Update, other)),
        }
    }

    /// Claim a parked [`Session::Uninstall`].
    ///
    /// # Errors
    /// As [`Self::claim_update`].
    pub fn claim_uninstall(&self) -> Result<(SessionClaim<'_>, Box<UninstallFlow>)> {
        match self.claim_one()? {
            (claim, Session::Uninstall(flow)) => Ok((claim, flow)),
            (claim, other) => Err(wrong_kind(claim, SessionKind::Uninstall, other)),
        }
    }

    /// Claim a parked [`Session::Rollback`].
    ///
    /// # Errors
    /// As [`Self::claim_update`].
    pub fn claim_rollback(&self) -> Result<(SessionClaim<'_>, Box<RollbackFlow>)> {
        match self.claim_one()? {
            (claim, Session::Rollback(flow)) => Ok((claim, flow)),
            (claim, other) => Err(wrong_kind(claim, SessionKind::Rollback, other)),
        }
    }
}

/// Put back a session the caller could not use, and say what was wrong.
///
/// Parking rather than dropping: the session is intact and its owner may still come back for it.
/// A command asking for the wrong kind is a bug in the frontend's sequencing, and losing the
/// user's preview to it would turn a mis-ordered call into lost work.
fn wrong_kind(claim: SessionClaim<'_>, want: SessionKind, found: Session) -> PdError {
    let found_kind = found.kind();
    claim.park(found);
    PdError::new(
        Code::IntUnexpected,
        format!(
            "expected {} session, found {}",
            want.as_str(),
            found_kind.as_str()
        ),
    )
}

/// A second plan was asked for while one is running.
fn in_flight() -> PdError {
    PdError::new(
        Code::ResPlanInFlight,
        "a plan is already resolving or executing",
    )
}

/// There is no parked plan — the UI called out of order, or a previous call already consumed it.
fn no_session() -> PdError {
    PdError::new(
        Code::IntUnexpected,
        "no plan is in progress; resolve one first",
    )
}

/// How far the in-memory name index has got.
///
/// **Measured on the real 864k-project index: `NameIndex::load` costs 140 ms in release** (572 ms
/// in debug, which is not the number that matters). Still roughly three times the entire
/// per-keystroke budget, and SP-3 had already ruled out the alternative — scanning SQLite per
/// keystroke measured 218 ms against 50 ms. So it is loaded once and held, and the only question
/// was when to pay for it. `crates/pipdock-core/tests/search_latency.rs` re-measures both.
///
/// The answer is on demand, with this state as the honest account of it: a user who never opens
/// Search never pays, and one who does gets the field immediately with a note rather than a
/// frozen window. Typing two or three characters takes about as long as the load, so in practice
/// the note is rarely seen — but `index_search` must **never** wait for it, which is what having
/// a state rather than an `Option<NameIndex>` and a lock enforces.
pub enum IndexSlot {
    /// Not asked for yet.
    Cold,
    /// A load is running. Searches answer `Warming` rather than blocking.
    Warming,
    /// Ready to search.
    Ready(Box<NameIndex>),
    /// The load failed — most often because the index has never been refreshed.
    Failed(String),
}

/// The last of the engine output, for a bug report (ERROR-CATALOG §4.1).
///
/// A plain `String` truncated at line boundaries rather than a ring of lines: what §4 needs is a
/// *tail under a byte budget*, and the budget is on the URL, not on a count of lines.
///
/// Fed from three places, not one. The obvious tap is `plan_execute`'s progress forwarder — but
/// the commonest thing a user reports is a plan that would not resolve, and a resolve emits no
/// progress events at all. So `plan_resolve` pushes the engine's own output on success and the
/// stderr tail on failure, which is exactly the case where a report is worth filing.
#[derive(Default)]
pub struct LogRing(std::sync::Mutex<String>);

impl LogRing {
    /// Append, trimming the front to the budget at a line boundary.
    pub fn push(&self, text: &str) {
        let Ok(mut buf) = self.0.lock() else { return };
        buf.push_str(text);
        if !text.ends_with('\n') {
            buf.push('\n');
        }
        if buf.len() <= pipdock_core::LOG_RING_BUFFER_BYTES {
            return;
        }
        // Cut at a char boundary first — a Vietnamese path or a package description would panic a
        // byte slice — then forward to the next newline so the buffer never opens mid-line.
        let over = buf.len() - pipdock_core::LOG_RING_BUFFER_BYTES;
        let start = (over..buf.len())
            .find(|i| buf.is_char_boundary(*i))
            .unwrap_or(buf.len());
        let trimmed = &buf[start..];
        *buf = match trimmed.find('\n') {
            Some(i) if i + 1 < trimmed.len() => trimmed[i + 1..].to_owned(),
            _ => trimmed.to_owned(),
        };
    }

    /// Everything held. Empty when nothing has run yet.
    pub fn read(&self) -> String {
        self.0.lock().map(|b| b.clone()).unwrap_or_default()
    }

    /// Start again — called when a new plan begins, so a report carries one run.
    pub fn clear(&self) {
        if let Ok(mut buf) = self.0.lock() {
            buf.clear();
        }
    }
}

/// Everything a command may need that outlives one call.
pub struct AppState {
    /// `%LOCALAPPDATA%\PipDock\data`.
    pub app_data: PathBuf,
    /// Settings, pins, recents and the package index.
    pub store: tokio::sync::Mutex<Store>,
    /// The 858k-name search index, once something has asked for it.
    ///
    /// A `std::sync::Mutex` rather than tokio's: it is only ever held to read or replace the slot,
    /// never across an await, and a search that had to await a lock would be a search that can be
    /// queued behind the 613 ms load — the exact thing this design exists to prevent.
    pub index: std::sync::Mutex<IndexSlot>,
    /// The mutation being driven across several IPC calls.
    pub sessions: Sessions<Session>,
    /// The Code Health run in flight, and the report it produced.
    ///
    /// **Not the mutation slot.** A health run touches no environment, so refusing it while an
    /// install streams would be a lock with no invariant behind it — and S5 bug 1 is the standing
    /// warning about claims that wedge. It still needs a slot of its own, because two Run clicks
    /// would put six subprocesses over one folder.
    ///
    /// `Sessions<HealthSession>` rather than `Sessions<()>` because P5's `health_fix` has to check
    /// the user's consent against a report **the server** is holding — the same shape `ack_ok`
    /// checks a `GuardAck` against the guard that produced it. Typed now so P5 does not redo it.
    pub health: Sessions<HealthSession>,

    /// The Security tab's slot — PRD P1-1.
    ///
    /// `Sessions<()>` because nothing is parked: an audit produces a report and hands it straight
    /// back, with no second call that consumes it the way `health_fix` consumes a `HealthSession`.
    /// What it is here for is the **token**, which lives beside the slot rather than in it so
    /// `audit_cancel` can reach a run the slot no longer owns.
    ///
    /// Its own slot rather than `health`'s, for the reason that one is not `sessions`: an audit
    /// touches no environment, so refusing it while Code Health runs would be a lock with no
    /// invariant behind it.
    pub audit: Sessions<()>,
    /// Engine output from the current plan, for the bug-report deep link.
    pub log: std::sync::Arc<LogRing>,
}

impl AppState {
    /// Open the store under the app-data directory.
    ///
    /// # Errors
    /// Propagates store failures, which at startup means the data directory is unusable.
    pub fn new() -> Result<Self> {
        let app_data = pipdock_core::store::default_app_data();
        let store = Store::open(&app_data)?;
        Ok(Self {
            app_data,
            store: tokio::sync::Mutex::new(store),
            index: std::sync::Mutex::new(IndexSlot::Cold),
            sessions: Sessions::default(),
            health: Sessions::default(),
            audit: Sessions::default(),
            log: std::sync::Arc::new(LogRing::default()),
        })
    }

    /// Claim the index slot for a load, if one is not already done or running.
    ///
    /// Returns `true` when the caller is now responsible for loading. Idempotent, so calling it on
    /// every Search render is harmless — which is what lets the screen ask without coordinating.
    pub fn begin_index_load(&self) -> bool {
        let Ok(mut slot) = self.index.lock() else {
            return false;
        };
        if matches!(*slot, IndexSlot::Cold | IndexSlot::Failed(_)) {
            *slot = IndexSlot::Warming;
            return true;
        }
        false
    }

    /// Publish the result of a load.
    pub fn finish_index_load(&self, loaded: pipdock_core::Result<NameIndex>) {
        if let Ok(mut slot) = self.index.lock() {
            *slot = match loaded {
                Ok(index) => IndexSlot::Ready(Box::new(index)),
                Err(e) => IndexSlot::Failed(e.message),
            };
        }
    }

    /// Search, without ever waiting for a load.
    ///
    /// `None` means "not ready" — the caller reports that as a state rather than an error, because
    /// warming is not a failure and a spinner that says so is better than a stall.
    pub fn search_index(&self, query: &str, limit: usize) -> Option<Vec<pipdock_core::index::Hit>> {
        let slot = self.index.lock().ok()?;
        match &*slot {
            IndexSlot::Ready(index) => Some(index.search(query, limit)),
            IndexSlot::Cold | IndexSlot::Warming | IndexSlot::Failed(_) => None,
        }
    }

    /// Why the index could not be loaded, when that is the state it is in.
    ///
    /// Reported as a state on the search result rather than as a command error: "the index has
    /// never been refreshed" is an action the user can take, and an error row would tell them
    /// something went wrong instead.
    pub fn index_failure(&self) -> Option<String> {
        match &*self.index.lock().ok()? {
            IndexSlot::Failed(why) => Some(why.clone()),
            IndexSlot::Cold | IndexSlot::Warming | IndexSlot::Ready(_) => None,
        }
    }

    /// Drop the loaded index, so the next search reloads it.
    ///
    /// Called after `index_refresh`: leaving the old names in memory would mean a refresh that
    /// reports thousands of new projects and a search that cannot find any of them.
    pub fn invalidate_index(&self) {
        if let Ok(mut slot) = self.index.lock() {
            *slot = IndexSlot::Cold;
        }
    }

    /// See [`Sessions::set_cancel`].
    pub fn set_cancel(&self, token: Option<CancellationToken>) {
        self.sessions.set_cancel(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The protocol is driven with a `u32` because an `UpdateFlow` cannot be constructed without a
    /// real interpreter to probe — and the protocol is what has the bugs, not the payload.
    fn sessions() -> Sessions<u32> {
        Sessions::default()
    }

    #[test]
    fn claiming_an_empty_slot_leaves_it_claimed_for_the_caller() {
        let s = sessions();
        let (claim, parked) = s.claim().expect("idle slot is claimable");
        assert!(parked.is_none());

        // `plan_resolve` is about to start work, so the slot must exclude a second resolve for the
        // whole of it — the empty case is a successful claim, not a miss.
        assert_eq!(
            s.claim().err().map(|e| e.code),
            Some(Code::ResPlanInFlight),
            "a second plan must be refused while the first holds the slot"
        );
        drop(claim);
        assert!(s.claim().is_ok(), "a dropped claim is a released slot");
    }

    #[test]
    fn a_claim_abandoned_by_an_early_return_releases_the_slot() {
        // U4. Three commands claimed and then had a `?` before their first release; a failed
        // settings read left the slot Busy and every later plan answered PD-RES-003.
        fn command(s: &Sessions<u32>) -> Result<()> {
            let (_claim, _) = s.claim()?;
            Err(PdError::new(Code::IntUnexpected, "store read failed"))?;
            Ok(())
        }
        let s = sessions();
        let token = CancellationToken::new();
        s.set_cancel(Some(token.clone()));
        assert!(command(&s).is_err());
        assert!(s.claim().is_ok(), "the next plan is not refused");
        assert!(!s.cancel_current(), "and the token went with the claim");
    }

    #[test]
    fn claiming_a_session_that_is_not_there_does_not_wedge_the_slot() {
        let s = sessions();
        assert_eq!(
            s.claim_one().err().map(|e| e.code),
            Some(Code::IntUnexpected)
        );
        // The regression this whole method exists for: the old code wrote `Busy` before
        // discovering there was nothing to take.
        s.park_for_test(7);
        assert_eq!(s.claim_one().ok().map(|(_, v)| v), Some(7));
    }

    #[test]
    fn a_failed_claim_clears_the_token_it_did_not_set() {
        let s = sessions();
        let token = CancellationToken::new();
        s.set_cancel(Some(token.clone()));
        let _ = s.claim_one();
        // Releasing means releasing: a stale token left behind would let the *next* plan's
        // `plan_cancel` report that it stopped something.
        assert!(!s.cancel_current(), "the stale token must be gone");
        assert!(!token.is_cancelled());
    }

    #[test]
    fn a_parked_session_survives_until_someone_claims_it() {
        let s = sessions();
        let (claim, _) = s.claim().expect("claim");
        claim.park(2);
        let (claim, value) = s.claim_one().expect("parked");
        assert_eq!(value, 2);
        claim.park(3);
        let (claim, value) = s.claim_one().expect("parked again");
        assert_eq!(value, 3);
        drop(claim);
        assert_eq!(
            s.claim_one().err().map(|e| e.code),
            Some(Code::IntUnexpected),
            "a claim dropped without parking discards the session"
        );
    }

    #[test]
    fn asking_for_the_wrong_kind_gives_it_back() {
        let s: Sessions<Session> = Sessions::default();
        s.park_for_test(Session::Fake(SessionKind::Uninstall));

        let err = s
            .claim_update()
            .err()
            .expect("an uninstall is not an update");
        assert_eq!(err.code, Code::IntUnexpected);
        assert!(
            err.message
                .contains("expected update session, found uninstall"),
            "the message must name both kinds, not just say 'no plan': {}",
            err.message
        );
        // Parked, not dropped: losing the user's guard dialog to a mis-ordered call would turn a
        // frontend sequencing bug into lost work.
        let (_, still_there) = s.claim().expect("slot is free");
        assert_eq!(
            still_there.map(|x| x.kind()),
            Some(SessionKind::Uninstall),
            "the refused session must survive the refusal"
        );
    }

    #[test]
    fn stop_discards_a_parked_session_and_cancels_a_running_one() {
        let s: Sessions<Session> = Sessions::default();
        assert!(!s.stop(), "nothing to stop");

        // Parked: no process exists, so stopping means letting go.
        s.park_for_test(Session::Fake(SessionKind::Update));
        assert!(s.stop());
        assert!(s.claim().expect("free").1.is_none());
    }

    #[test]
    fn cancel_reaches_a_session_the_slot_no_longer_owns() {
        let s = sessions();
        let token = CancellationToken::new();
        s.park_for_test(1);
        let (claim, _) = s.claim_one().expect("parked");
        s.set_cancel(Some(token.clone()));

        // The slot is `Busy` and the session is owned by the caller — which is exactly when
        // `plan_cancel` has to work, and why the token is not stored inside the slot.
        assert!(s.stop());
        assert!(token.is_cancelled());
        drop(claim);
        assert!(s.claim().is_ok(), "the owner's drop is what frees it");
    }
}
