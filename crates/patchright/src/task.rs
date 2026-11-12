//! Task spawning for the crate: every task goes through one of these two,
//! so whether it inherits the caller's span is decided at the call site.

use std::future::Future;
use tracing::Instrument;

/// Spawns `future` on the runtime inside the caller's current span.
///
/// Clippy's `disallowed-methods` refuses a bare `tokio::spawn` everywhere
/// else in the crate, so an event handler spawned from the read loop always
/// runs under its `dispatch` span. A task that outlives the call starting it
/// uses [`spawn_detached`] instead.
pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    spawn_detached(future.in_current_span())
}

/// Spawns `future` with no parent span, for a task that outlives the call
/// starting it: a read loop, the driver's stderr drain, the SIGINT listener.
/// Inheriting the caller's span would hold it open as long as the task runs.
#[allow(clippy::disallowed_methods)]
pub(crate) fn spawn_detached<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}
