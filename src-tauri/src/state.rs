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
//! seconds later. Something has to hold the flow in between, and that is [`PlanSlot`].

use std::path::PathBuf;

use pipdock_core::errors::{Code, PdError, Result};
use pipdock_core::flow::UpdateFlow;
use pipdock_core::index::NameIndex;
use pipdock_core::store::Store;
use tokio_util::sync::CancellationToken;

/// Where the one plan this session may have has got to.
///
/// One at a time, deliberately. Two concurrent plans would interleave engine commands against a
/// single environment, and DATA-FLOW §9.3's staleness check would be comparing the installed set
/// against one the other plan is busy changing — so the second is refused with `PD-RES-003`
/// rather than allowed to race.
pub enum PlanSlot {
    /// No plan.
    Idle,
    /// A flow is waiting for the user: a decision, or the final confirm.
    Ready(Box<UpdateFlow>),
    /// Engine work is in flight, so the flow is owned by whichever command is driving it.
    ///
    /// The flow is *moved out* for the duration rather than borrowed, because `execute` awaits for
    /// as long as the install takes and holding the slot's lock across that would block
    /// `plan_cancel` — the one command that must work precisely then.
    Busy,
}

impl PlanSlot {
    /// True while engine work is in flight.
    pub const fn is_busy(&self) -> bool {
        matches!(*self, Self::Busy)
    }
}

/// How far the in-memory name index has got.
///
/// **Measured on the real 858k-project index: `NameIndex::load` costs 613 ms** — twelve times the
/// entire per-keystroke budget, and SP-3 had already ruled out the alternative (scanning SQLite
/// per keystroke measured 218 ms against 50 ms). So it is loaded once and held, and the only
/// question was when to pay for it.
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

/// Everything a command may need that outlives one call.
pub struct AppState {
    /// `%LOCALAPPDATA%\PipDock`.
    pub app_data: PathBuf,
    /// Settings, pins, recents and the package index.
    pub store: tokio::sync::Mutex<Store>,
    /// The 858k-name search index, once something has asked for it.
    ///
    /// A `std::sync::Mutex` rather than tokio's: it is only ever held to read or replace the slot,
    /// never across an await, and a search that had to await a lock would be a search that can be
    /// queued behind the 613 ms load — the exact thing this design exists to prevent.
    pub index: std::sync::Mutex<IndexSlot>,
    /// The plan being driven across several IPC calls.
    pub plan: tokio::sync::Mutex<PlanSlot>,
    /// The token for whatever engine work is in flight.
    ///
    /// Deliberately **not** inside [`PlanSlot`]: `plan_cancel` must be able to trip it while the
    /// slot is `Busy` and its flow is owned elsewhere. Its own lock is only ever held for the
    /// moment it takes to read or replace an `Option`, never across an await.
    pub cancel: std::sync::Mutex<Option<CancellationToken>>,
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
            plan: tokio::sync::Mutex::new(PlanSlot::Idle),
            cancel: std::sync::Mutex::new(None),
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

    /// Claim the plan slot for engine work, returning whatever flow was waiting there.
    ///
    /// # Errors
    /// `PD-RES-003` when a plan is already in flight.
    pub async fn claim(&self) -> Result<Option<Box<UpdateFlow>>> {
        let mut slot = self.plan.lock().await;
        if slot.is_busy() {
            return Err(PdError::new(
                Code::ResPlanInFlight,
                "a plan is already resolving or executing",
            ));
        }
        Ok(match std::mem::replace(&mut *slot, PlanSlot::Busy) {
            PlanSlot::Ready(flow) => Some(flow),
            PlanSlot::Idle | PlanSlot::Busy => None,
        })
    }

    /// Put a flow back for the next call to pick up.
    pub async fn park(&self, flow: Box<UpdateFlow>) {
        *self.plan.lock().await = PlanSlot::Ready(flow);
    }

    /// Release the slot with no flow — the plan finished, or failed.
    ///
    /// Called on **every** exit path out of a claimed slot. A claim that is never released leaves
    /// the session permanently refusing plans with `PD-RES-003`, which is the failure mode this
    /// design has to be careful about.
    pub async fn release(&self) {
        *self.plan.lock().await = PlanSlot::Idle;
        self.set_cancel(None);
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
}
