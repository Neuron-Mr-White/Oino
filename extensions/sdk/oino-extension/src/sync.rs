//! Single-threaded synchronisation for state shared across concurrent guest calls.
//!
//! Components run on one thread with cooperative tasks; use [`std::cell::RefCell`] for state
//! never held across `.await`, and [`Mutex`] when it must be.

pub use futures::lock::{Mutex, MutexGuard};
use std::cell::RefCell;
use std::collections::HashMap;

/// Per-session state keyed by [`crate::context::Ctx::session_id`].
pub struct SessionMap<T> {
    inner: RefCell<HashMap<String, T>>,
}

impl<T> Default for SessionMap<T> {
    fn default() -> Self {
        Self {
            inner: RefCell::new(HashMap::new()),
        }
    }
}

impl<T: Default> SessionMap<T> {
    /// Runs `f` on the state of `session_id` (created on first use).
    pub fn with<R>(&self, session_id: &str, f: impl FnOnce(&mut T) -> R) -> R {
        let mut map = self.inner.borrow_mut();
        f(map.entry(session_id.to_owned()).or_default())
    }

    /// Drops the state of `session_id`.
    pub fn remove(&self, session_id: &str) {
        self.inner.borrow_mut().remove(session_id);
    }
}
