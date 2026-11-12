//! In-flight accounting for route handlers, so `unroute_all` can honor its
//! `behavior` argument: wait for the invocations already running, or tell
//! them to keep quiet about errors.

use crate::protocol::route::UnrouteBehavior;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// The running invocations of one route handler.
///
/// Every invocation holds an [`Active`] guard for its duration; dropping the
/// last guard wakes anyone in [`wait_idle`](Self::wait_idle).
#[derive(Default)]
pub(crate) struct InFlight {
    active: Mutex<usize>,
    idle: Notify,
    ignore_errors: AtomicBool,
}

/// Marks one invocation as running until dropped.
pub(crate) struct Active(Arc<InFlight>);

impl InFlight {
    /// Records the start of an invocation. Keep the guard alive while the
    /// handler runs.
    pub(crate) fn enter(self: &Arc<Self>) -> Active {
        *self.active.lock().unwrap() += 1;
        Active(self.clone())
    }

    /// Resolves once no invocation is running. Returns at once if none is.
    pub(crate) async fn wait_idle(&self) {
        loop {
            // Register before checking, so a guard dropped in between still
            // wakes this waiter.
            let notified = self.idle.notified();
            if *self.active.lock().unwrap() == 0 {
                return;
            }
            notified.await;
        }
    }

    /// Asks the running invocations to swallow their errors.
    pub(crate) fn ignore_errors(&self) {
        self.ignore_errors.store(true, Ordering::SeqCst);
    }

    /// Whether errors from this handler should go unreported.
    pub(crate) fn ignores_errors(&self) -> bool {
        self.ignore_errors.load(Ordering::SeqCst)
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        let mut active = self.0.active.lock().unwrap();
        *active -= 1;
        if *active == 0 {
            self.0.idle.notify_waiters();
        }
    }
}

/// Applies `unroute_all`'s behavior to the handlers just removed: `Wait`
/// blocks until their running invocations finish, `IgnoreErrors` lets those
/// invocations fail silently, and the default does neither.
pub(crate) async fn settle_removed<'a>(
    removed: impl IntoIterator<Item = &'a Arc<InFlight>>,
    behavior: Option<UnrouteBehavior>,
) {
    match behavior.unwrap_or(UnrouteBehavior::Default) {
        UnrouteBehavior::Wait => {
            for handler in removed {
                handler.wait_idle().await;
            }
        }
        UnrouteBehavior::IgnoreErrors => {
            for handler in removed {
                handler.ignore_errors();
            }
        }
        UnrouteBehavior::Default => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn wait_idle_returns_at_once_when_nothing_runs() {
        let flight = Arc::new(InFlight::default());
        timeout(Duration::from_millis(100), flight.wait_idle())
            .await
            .expect("idle tracker must not block");
    }

    #[tokio::test]
    async fn wait_idle_blocks_until_the_last_guard_drops() {
        let flight = Arc::new(InFlight::default());
        let first = flight.enter();
        let second = flight.enter();
        assert!(
            timeout(Duration::from_millis(50), flight.wait_idle())
                .await
                .is_err(),
            "two invocations are running"
        );
        drop(first);
        assert!(
            timeout(Duration::from_millis(50), flight.wait_idle())
                .await
                .is_err(),
            "one invocation is still running"
        );
        let waiter = crate::task::spawn({
            let flight = flight.clone();
            async move { flight.wait_idle().await }
        });
        drop(second);
        timeout(Duration::from_secs(1), waiter)
            .await
            .expect("dropping the last guard wakes the waiter")
            .unwrap();
    }

    #[tokio::test]
    async fn settle_wait_returns_only_after_every_removed_handler_is_idle() {
        let a = Arc::new(InFlight::default());
        let b = Arc::new(InFlight::default());
        let running = b.enter();
        let removed = [a.clone(), b.clone()];
        let settled = crate::task::spawn(async move {
            settle_removed(removed.iter(), Some(UnrouteBehavior::Wait)).await;
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!settled.is_finished(), "b still has a running invocation");
        drop(running);
        timeout(Duration::from_secs(1), settled)
            .await
            .expect("settle completes once b is idle")
            .unwrap();
        assert!(!a.ignores_errors() && !b.ignores_errors());
    }

    #[tokio::test]
    async fn settle_ignore_errors_flags_every_removed_handler_without_waiting() {
        let a = Arc::new(InFlight::default());
        let b = Arc::new(InFlight::default());
        let _running = b.enter();
        timeout(
            Duration::from_millis(100),
            settle_removed([&a, &b], Some(UnrouteBehavior::IgnoreErrors)),
        )
        .await
        .expect("ignore-errors must not wait on the running invocation");
        assert!(a.ignores_errors());
        assert!(b.ignores_errors());
    }

    #[tokio::test]
    async fn settle_default_neither_waits_nor_flags() {
        let a = Arc::new(InFlight::default());
        let _running = a.enter();
        for behavior in [None, Some(UnrouteBehavior::Default)] {
            timeout(Duration::from_millis(100), settle_removed([&a], behavior))
                .await
                .expect("default behavior returns at once");
        }
        assert!(!a.ignores_errors());
    }
}
