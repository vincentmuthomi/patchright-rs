//! JSON-RPC Connection layer for Playwright protocol

use crate::error::{Error, Result};
use crate::server::transport::{TransportReceiver, TransportSender};
use parking_lot::Mutex as ParkingLotMutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::{mpsc, oneshot};

use crate::protocol::selectors::Selectors;
use crate::server::channel_owner::ChannelOwner;
use crate::server::error_parsing::parse_protocol_error;

/// Trait defining the interface that ChannelOwner needs from a Connection.
///
/// Uses `#[async_trait]` for ergonomic async method definitions while
/// maintaining dyn-safety (`Arc<dyn ConnectionLike>`).
#[async_trait::async_trait]
pub trait ConnectionLike: Send + Sync {
    /// Send a message to the Playwright server and await response.
    async fn send_message(&self, guid: &str, method: &str, params: Value) -> Result<Value>;

    /// Register an object in the connection's registry.
    async fn register_object(&self, guid: Arc<str>, object: Arc<dyn ChannelOwner>);

    /// Unregister an object from the connection's registry.
    async fn unregister_object(&self, guid: &str);

    /// Unregister an object synchronously.
    ///
    /// The registry is a synchronous lock, so this needs no async context;
    /// it exists so `dispose()` (a sync path) can unregister immediately
    /// instead of spawning a task and leaving a window where a disposed
    /// object still receives events.
    fn unregister_object_sync(&self, guid: &str);

    /// Get an object by GUID.
    async fn get_object(&self, guid: &str) -> Result<Arc<dyn ChannelOwner>>;

    /// Returns all objects currently registered in the connection's registry.
    ///
    /// Used for operations that need to search across all known objects,
    /// such as finding child frames by matching their `parentFrame` GUID.
    async fn get_all_objects(&self) -> Vec<Arc<dyn ChannelOwner>>;

    /// Returns all objects currently registered in the connection's registry, synchronously.
    ///
    /// This is a synchronous variant for use in non-async contexts such as `child_frames()`.
    /// Callers that hold a reference to `Arc<dyn ConnectionLike>` can call this without
    /// needing to enter an async context.
    fn all_objects_sync(&self) -> Vec<Arc<dyn ChannelOwner>>;

    /// Returns the shared Selectors coordinator for this connection.
    ///
    /// Selectors is a pure client-side object that tracks custom selector
    /// engine registrations and the test ID attribute, propagating them to
    /// all BrowserContext instances.
    fn selectors(&self) -> Arc<Selectors>;

    /// Waits for an object with `guid` to appear in the registry, with a 30s
    /// deadline.
    ///
    /// The `__create__` for an object referenced in a response can arrive
    /// just after the response itself, so "not in the registry yet" is a
    /// race, not an error. This waits on the registration notification
    /// rather than polling, so resolution is immediate once the object
    /// lands.
    async fn wait_for_object(&self, guid: &str) -> Result<Arc<dyn ChannelOwner>>;

    /// Close the transport writer, signalling EOF on the driver's stdin.
    ///
    /// This is the driver's sanctioned shutdown: `run-driver` wires
    /// `transport.onclose` to `gracefullyProcessExitDoNotHang`, which closes
    /// every browser it owns before exiting. Sending the driver a signal is
    /// not a substitute — see [`Playwright::drop`](crate::protocol::Playwright).
    ///
    /// Synchronous so it can run from `Drop`. Returns `false` if the writer
    /// was already closed, or if a send is in flight and holds the lock.
    fn close_writer(&self) -> bool;
}

/// Extension trait for typed object retrieval from a connection.
///
/// Provides `get_typed::<T>(guid)` which combines `get_object` + downcast
/// into a single call with a proper `TypeMismatch` error.
///
/// # Example
///
/// ```no_run
/// # use std::sync::Arc;
/// # use patchright_rs::protocol::Page;
/// # use patchright_rs::server::connection::ConnectionLike;
/// use patchright_rs::server::connection::ConnectionExt;
///
/// # async fn example(connection: Arc<dyn ConnectionLike>, guid: String) -> patchright_rs::Result<()> {
/// let page: Page = connection.get_typed::<Page>(&guid).await?;
/// # Ok(())
/// # }
/// ```
#[async_trait::async_trait]
pub trait ConnectionExt: ConnectionLike {
    /// Get an object by GUID and downcast to the expected concrete type.
    ///
    /// Returns `Error::TypeMismatch` if the object exists but is not of type `T`.
    async fn get_typed<T: ChannelOwner + Clone + 'static>(&self, guid: &str) -> Result<T> {
        let obj = self.get_object(guid).await?;
        obj.as_any()
            .downcast_ref::<T>()
            .cloned()
            .ok_or_else(|| Error::TypeMismatch {
                guid: guid.to_string(),
                expected: std::any::type_name::<T>().to_string(),
                actual: obj.type_name().to_string(),
            })
    }

    /// Waits for the object to land in the registry, then downcasts to `T`.
    ///
    /// Use this to resolve a guid referenced in a response: its `__create__`
    /// can arrive just after the response itself, so absence is a race and is
    /// waited out (event-driven, 30s deadline). A wrong type, by contrast, is
    /// deterministic — retrying it can never succeed — so `TypeMismatch`
    /// fails immediately. Replaces the 20x50ms polling loops that treated
    /// both cases alike, burning a second before reporting the mismatch.
    async fn wait_for_typed<T: ChannelOwner + Clone + 'static>(&self, guid: &str) -> Result<T> {
        let obj = self.wait_for_object(guid).await?;
        obj.as_any()
            .downcast_ref::<T>()
            .cloned()
            .ok_or_else(|| Error::TypeMismatch {
                guid: guid.to_string(),
                expected: std::any::type_name::<T>().to_string(),
                actual: obj.type_name().to_string(),
            })
    }
}

// Blanket implementation: any ConnectionLike automatically gets ConnectionExt.
impl<C: ConnectionLike + ?Sized> ConnectionExt for C {}

/// Downcast a protocol object's parent to a concrete type.
///
/// Returns `None` if the object has no parent or the parent is not of type `T`.
///
/// # Example
///
/// ```no_run
/// # use std::sync::Arc;
/// # use patchright_rs::protocol::Page;
/// # use patchright_rs::server::channel_owner::ChannelOwner;
/// # use patchright_rs::server::connection::downcast_parent;
/// # fn example(dialog_arc: Arc<dyn ChannelOwner>) {
/// let page: Option<Page> = downcast_parent::<Page>(&*dialog_arc);
/// # }
/// ```
pub fn downcast_parent<T: ChannelOwner + Clone + 'static>(obj: &dyn ChannelOwner) -> Option<T> {
    obj.parent()
        .and_then(|p| p.as_any().downcast_ref::<T>().cloned())
}

/// Metadata attached to every Playwright protocol message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(rename = "wallTime")]
    pub wall_time: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Per-call action timeout, in milliseconds.
    ///
    /// Playwright 1.62 moved `timeout` out of every method's params and into
    /// this envelope (`packages/protocol/spec/core.yml`). A server from 1.62
    /// on ignores a `timeout` left in params, and with none here it waits
    /// indefinitely rather than falling back to a default, so a call that
    /// should fail fast hangs instead. [`Connection::send_message`] relocates
    /// it; option structs still carry `timeout` because that is Playwright's
    /// public API in every binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<i32>,
}

impl Metadata {
    pub fn now() -> Self {
        Self {
            wall_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is before UNIX epoch")
                .as_millis() as i64,
            internal: Some(false),
            location: None,
            title: None,
            timeout: None,
        }
    }
}

/// Moves a `timeout` param into the metadata envelope, where the driver reads
/// it from Playwright 1.62 on.
///
/// Returns the timeout if there was one, having removed it from `params`.
/// Matches playwright-python, which does the same `params.pop("timeout")` in
/// its connection layer rather than threading a timeout through every call
/// signature. No 1.62 method declares a `timeout` param any more, so nothing
/// else can be caught by the key.
fn take_timeout(params: &mut Value) -> Option<f64> {
    params
        .as_object_mut()
        .and_then(|obj| obj.remove("timeout"))
        .and_then(|v| v.as_f64())
}

/// Four random lowercase letters that name this connection in its call ids,
/// the `abcd@7` shape of the JS and Python clients. Since 1.64 the driver
/// takes a request's `id` verbatim as the call id it writes into traces, and
/// the trace format types that id as a string.
fn call_id_prefix() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bits = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    (0..4)
        .map(|_| {
            let letter = b'a' + (bits % 26) as u8;
            bits /= 26;
            letter as char
        })
        .collect()
}

/// Protocol request message sent to Playwright server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    #[serde(
        serialize_with = "serialize_arc_str",
        deserialize_with = "deserialize_arc_str"
    )]
    pub guid: Arc<str>,
    pub method: String,
    #[serde(skip_serializing_if = "is_value_null")]
    pub params: Value,
    pub metadata: Metadata,
}

fn is_value_null(v: &Value) -> bool {
    v.is_null()
}

pub fn serialize_arc_str<S>(arc: &Arc<str>, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(arc)
}

pub fn deserialize_arc_str<'de, D>(deserializer: D) -> std::result::Result<Arc<str>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = serde::Deserialize::deserialize(deserializer)?;
    Ok(Arc::from(s.as_str()))
}

/// Protocol response message from Playwright server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorWrapper>,
    /// Failure details, a top-level sibling of `error`. Populated for methods
    /// that declare `errorDetails` in the protocol (currently only
    /// `Frame.expect`, used for auto-retrying assertions). Kept as raw JSON, not
    /// a typed struct, so a future driver's shape change can never fail the whole
    /// message parse (which would drop the message and hang the caller);
    /// `parse_protocol_error` interprets it best-effort.
    #[serde(rename = "errorDetails", skip_serializing_if = "Option::is_none")]
    pub error_details: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorWrapper {
    pub error: ErrorPayload,
}

/// Structured `expect` failure details (Playwright 1.61+). Sent alongside an
/// `error`. The driver attaches this to *every* `Frame.expect` error, but only
/// a genuine assertion result populates a field — an infrastructure error (e.g.
/// the target closing mid-retry) arrives with an empty `{}`, so classification
/// keys on field content, not the object's presence (see `parse_protocol_error`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpectErrorDetails {
    #[serde(default)]
    pub timed_out: Option<bool>,
    #[serde(default)]
    pub custom_error_message: Option<String>,
    #[serde(default)]
    pub received: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(
        serialize_with = "serialize_arc_str",
        deserialize_with = "deserialize_arc_str"
    )]
    pub guid: Arc<str>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Response(Response),
    Event(Event),
}

type ObjectRegistry = HashMap<Arc<str>, Arc<dyn ChannelOwner>>;

/// JSON-RPC connection to Playwright server
pub struct Connection {
    next_ordinal: AtomicU32,
    call_id_prefix: String,
    /// Pending request callbacks. Sync lock: only ever held for an
    /// insert/remove, never across an await point.
    callbacks: Arc<ParkingLotMutex<HashMap<String, oneshot::Sender<Result<Value>>>>>,
    /// Transport writer. A tokio (async-aware) mutex on purpose: the guard is
    /// held across the `send().await`, which is what serializes whole-message
    /// writes onto the single pipe/socket.
    sender: Arc<TokioMutex<Option<Box<dyn TransportSender>>>>,
    message_rx: Arc<TokioMutex<Option<mpsc::Receiver<Value>>>>,
    transport_receiver: Arc<TokioMutex<Option<Box<dyn TransportReceiver>>>>,
    objects: Arc<ParkingLotMutex<ObjectRegistry>>,
    /// Signalled whenever an object is registered; lets `wait_for_object`
    /// block on creation instead of polling.
    object_created: tokio::sync::Notify,
    /// Shared Selectors coordinator for this connection.
    ///
    /// Selectors is a client-side object; it is created once per Connection and
    /// wired into every BrowserContext that is created on this connection.
    selectors: Arc<Selectors>,
    /// Identity of the tokio runtime that constructed this connection.
    /// Used in debug builds to fail fast when a `Browser` is shared
    /// across runtimes (e.g. between `#[tokio::test]` invocations) —
    /// the protocol channels are bound to the original runtime, so
    /// cross-runtime use silently deadlocks. `None` when constructed
    /// outside any runtime.
    #[cfg(debug_assertions)]
    creator_runtime: Option<tokio::runtime::Id>,
}

// Type alias for compatibility (though generic parameters are gone, we can keep alias if needed)
pub type RealConnection = Connection;

impl Connection {
    pub fn new(
        sender: impl TransportSender + 'static,
        receiver: impl TransportReceiver + 'static,
        message_rx: mpsc::Receiver<Value>,
    ) -> Self {
        Self {
            next_ordinal: AtomicU32::new(0),
            call_id_prefix: call_id_prefix(),
            callbacks: Arc::new(ParkingLotMutex::new(HashMap::new())),
            sender: Arc::new(TokioMutex::new(Some(Box::new(sender)))),
            message_rx: Arc::new(TokioMutex::new(Some(message_rx))),
            transport_receiver: Arc::new(TokioMutex::new(Some(Box::new(receiver)))),
            objects: Arc::new(ParkingLotMutex::new(HashMap::new())),
            object_created: tokio::sync::Notify::new(),
            selectors: Arc::new(Selectors::new()),
            #[cfg(debug_assertions)]
            creator_runtime: tokio::runtime::Handle::try_current().ok().map(|h| h.id()),
        }
    }

    /// Panic in debug builds when this connection is being used from a
    /// different tokio runtime than the one that created it. No-op in
    /// release. Skips the check entirely when no creator runtime was
    /// captured (the connection was constructed outside any runtime —
    /// rare, only relevant in unit tests with stubbed transports).
    fn assert_same_runtime(&self) {
        #[cfg(debug_assertions)]
        {
            let Some(creator) = self.creator_runtime else {
                return;
            };
            let Some(current) = tokio::runtime::Handle::try_current().ok().map(|h| h.id()) else {
                return;
            };
            assert_eq!(
                current, creator,
                "playwright-rs: Browser used from a different tokio runtime than the one that \
                 launched it. The protocol channels are bound to the launching runtime; using \
                 a Browser from another runtime (e.g. sharing across `#[tokio::test]` boundaries \
                 via OnceCell) silently deadlocks. Launch a fresh Playwright + Browser per \
                 runtime instead."
            );
        }
    }

    #[tracing::instrument(
        level = "trace",
        skip_all,
        fields(guid = %guid, method = %method, id = tracing::field::Empty)
    )]
    pub async fn send_message(
        &self,
        guid: String,
        method: String,
        mut params: Value,
    ) -> Result<Value> {
        self.assert_same_runtime();

        let ordinal = self.next_ordinal.fetch_add(1, Ordering::SeqCst);
        let id = format!("{}@{ordinal}", self.call_id_prefix);
        tracing::Span::current().record("id", id.as_str());

        let (tx, rx) = oneshot::channel();
        self.callbacks.lock().insert(id.clone(), tx);

        let mut metadata = Metadata::now();
        metadata.timeout = take_timeout(&mut params);

        let request = Request {
            id,
            guid: Arc::from(guid),
            method,
            params,
            metadata,
        };

        let request_value = serde_json::to_value(&request)?;
        tracing::trace!("Request JSON: {}", request_value);

        {
            let mut guard = self.sender.lock().await;
            let writer = guard.as_mut().ok_or(Error::ChannelClosed)?;
            match writer.send(request_value).await {
                Ok(()) => tracing::trace!("Message sent successfully, awaiting response"),
                Err(e) => {
                    tracing::error!("Failed to send message: {:?}", e);
                    return Err(e);
                }
            }
        }

        tracing::trace!("Waiting for response to ID {}", request.id);
        rx.await
            .map_err(|_| Error::ChannelClosed)
            .and_then(|result| result)
    }

    pub async fn initialize_playwright(self: &Arc<Self>) -> Result<Arc<dyn ChannelOwner>> {
        use crate::protocol::Root;
        use std::time::Duration;

        let root = Arc::new(Root::new(Arc::clone(self) as Arc<dyn ConnectionLike>))
            as Arc<dyn ChannelOwner>;

        self.objects.lock().insert(Arc::from(""), root.clone());

        tracing::debug!("Root object registered, sending initialize message");

        let root_typed: Root = self.get_typed::<Root>("").await?;

        let response = tokio::time::timeout(Duration::from_secs(30), root_typed.initialize())
            .await
            .map_err(|_| {
                Error::Timeout("Playwright initialization timeout after 30 seconds".to_string())
            })??;

        let playwright_guid = response["playwright"]["guid"].as_str().ok_or_else(|| {
            Error::ProtocolError("Initialize response missing 'playwright.guid' field".to_string())
        })?;

        tracing::debug!("Initialized Playwright with GUID: {}", playwright_guid);

        let playwright_obj = self.wait_for_object(playwright_guid).await?;

        // Validate that the object is indeed a Playwright instance
        let _: crate::protocol::Playwright = self
            .get_typed::<crate::protocol::Playwright>(playwright_guid)
            .await?;

        let empty_guid: Arc<str> = Arc::from("");
        self.objects.lock().remove(&empty_guid);
        tracing::debug!("Root object unregistered after successful initialization");

        Ok(playwright_obj)
    }

    pub async fn run(self: &Arc<Self>) {
        let mut transport_receiver = self
            .transport_receiver
            .lock()
            .await
            .take()
            .expect("run() can only be called once - transport receiver already taken");

        let transport_handle = crate::task::spawn_detached(async move {
            if let Err(e) = transport_receiver.run().await {
                tracing::error!("Transport error: {}", e);
            }
        });

        let mut message_rx = self
            .message_rx
            .lock()
            .await
            .take()
            .expect("run() can only be called once - message receiver already taken");

        while let Some(message_value) = message_rx.recv().await {
            match serde_json::from_value::<Message>(message_value) {
                Ok(message) => {
                    if let Err(e) = self.dispatch_internal(message).await {
                        tracing::error!("Error dispatching message: {}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to parse message: {}", e);
                }
            }
        }

        tracing::debug!("Message loop ended (transport closed)");
        let _ = transport_handle.await;
    }

    #[cfg(test)]
    pub async fn dispatch(self: &Arc<Self>, message: Message) -> Result<()> {
        self.dispatch_internal(message).await
    }

    #[tracing::instrument(name = "dispatch", level = "trace", skip_all)]
    async fn dispatch_internal(self: &Arc<Self>, message: Message) -> Result<()> {
        tracing::trace!("Dispatching message: {:?}", message);
        match message {
            Message::Response(response) => {
                tracing::trace!("Processing response for ID: {}", response.id);
                let callback = self.callbacks.lock().remove(&response.id).ok_or_else(|| {
                    Error::ProtocolError(format!(
                        "Cannot find request to respond: id={}",
                        response.id
                    ))
                })?;

                let result = if let Some(error_wrapper) = response.error {
                    Err(parse_protocol_error(
                        error_wrapper.error,
                        response.error_details,
                    ))
                } else {
                    Ok(response.result.unwrap_or(Value::Null))
                };

                let _ = callback.send(result);
                Ok(())
            }
            Message::Event(event) => match event.method.as_str() {
                "__create__" => self.handle_create(&event).await,
                "__dispose__" => self.handle_dispose(&event).await,
                "__adopt__" => self.handle_adopt(&event).await,
                _ => match self.objects.lock().get(&event.guid).cloned() {
                    Some(object) => {
                        object.on_event(&event.method, event.params);
                        Ok(())
                    }
                    None => {
                        tracing::warn!(
                            "Event for unknown object: guid={}, method={}",
                            event.guid,
                            event.method
                        );
                        Ok(())
                    }
                },
            },
        }
    }

    async fn handle_create(self: &Arc<Self>, event: &Event) -> Result<()> {
        use crate::server::channel_owner::ParentOrConnection;
        use crate::server::object_factory::create_object;

        let type_name = event.params["type"]
            .as_str()
            .ok_or_else(|| Error::ProtocolError("__create__ missing 'type'".to_string()))?
            .to_string();

        let object_guid: Arc<str> = Arc::from(
            event.params["guid"]
                .as_str()
                .ok_or_else(|| Error::ProtocolError("__create__ missing 'guid'".to_string()))?,
        );

        tracing::trace!(
            "__create__: type={}, guid={}, parent_guid={}",
            type_name,
            object_guid,
            event.guid
        );

        let initializer = event.params["initializer"].clone();

        let parent_or_conn = if event.guid.is_empty() {
            ParentOrConnection::Connection(Arc::clone(self) as Arc<dyn ConnectionLike>)
        } else {
            let parent_obj = self
                .objects
                .lock()
                .get(&event.guid)
                .cloned()
                .ok_or_else(|| {
                    tracing::error!(
                        "DEBUG: Parent object not found for type={}, parent_guid={}",
                        type_name,
                        event.guid
                    );
                    Error::ProtocolError(format!("Parent object not found: {}", event.guid))
                })?;
            ParentOrConnection::Parent(parent_obj)
        };

        let object = match create_object(
            parent_or_conn.clone(),
            type_name.clone(),
            object_guid.clone(),
            initializer,
        )
        .await
        {
            Ok(obj) => obj,
            Err(e) => {
                tracing::error!(
                    "DEBUG: Failed to create object type={}, guid={}, error={}",
                    type_name,
                    object_guid,
                    e
                );
                return Err(e);
            }
        };

        self.register_object(Arc::clone(&object_guid), object.clone())
            .await;

        if let ParentOrConnection::Parent(parent) = &parent_or_conn {
            parent.add_child(Arc::clone(&object_guid), object);
        }

        tracing::trace!(
            "Successfully created and registered object: type={}, guid={}",
            type_name,
            object_guid
        );

        Ok(())
    }

    async fn handle_dispose(self: &Arc<Self>, event: &Event) -> Result<()> {
        use crate::server::channel_owner::DisposeReason;

        let guid = &event.guid;

        // Find object - check lock in scope
        let object = { self.objects.lock().get(guid).cloned() };

        if let Some(object) = object {
            // Unregister from connection
            self.unregister_object(guid).await;

            // Remove from parent
            if let Some(parent) = object.parent() {
                parent.remove_child(guid);
            }

            // Dispose object
            object.dispose(DisposeReason::Protocol);

            tracing::trace!("Disposed object: guid={}", guid);
        } else {
            tracing::trace!("Ignoring __dispose__ for unknown object: guid={}", guid);
        }

        Ok(())
    }

    pub async fn wait_for_object(&self, guid: &str) -> Result<Arc<dyn ChannelOwner>> {
        use tokio::time::{Duration, Instant, timeout_at};
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            // Create the Notified future BEFORE checking the registry.
            // Tokio guarantees a created-but-unpolled `Notified` receives
            // `notify_waiters` wakeups, so a registration landing between
            // the check and the first poll still wakes this task. (That
            // guarantee is specific to creation order; `notify_waiters`
            // stores no permit the way `notify_one` does.)
            let created = self.object_created.notified();
            if let Some(obj) = self.objects.lock().get(guid) {
                return Ok(obj.clone());
            }
            if timeout_at(deadline, created).await.is_err() {
                return Err(Error::Timeout(format!(
                    "Timed out waiting for object {}",
                    guid
                )));
            }
        }
    }

    async fn handle_adopt(self: &Arc<Self>, event: &Event) -> Result<()> {
        let child_guid = event.params["guid"]
            .as_str()
            .ok_or_else(|| Error::ProtocolError("__adopt__ missing 'guid'".to_string()))?;

        let new_parent_guid = &event.guid;

        let child = self.get_object(child_guid).await.map_err(|e| {
            Error::ProtocolError(format!("Child object not found during adopt: {}", e))
        })?;

        let new_parent = self.get_object(new_parent_guid).await.map_err(|e| {
            Error::ProtocolError(format!("Parent object not found during adopt: {}", e))
        })?;

        // 1. Remove from old parent
        if let Some(old_parent) = child.parent() {
            old_parent.remove_child(child_guid);
        }

        // 2. Add to new parent (this will update child's weak parent ref if we had mutable access,
        // but since we only have Arc<dyn ChannelOwner>, ChannelOwner logic needs to handle it.
        // Actually, ChannelOwner trait has `adopt` method.
        new_parent.adopt(child);

        Ok(())
    }
}

#[async_trait::async_trait]
impl ConnectionLike for Connection {
    async fn send_message(&self, guid: &str, method: &str, params: Value) -> Result<Value> {
        self.send_message(guid.to_string(), method.to_string(), params)
            .await
    }

    async fn register_object(&self, guid: Arc<str>, object: Arc<dyn ChannelOwner>) {
        self.objects.lock().insert(guid, object);
        self.object_created.notify_waiters();
    }

    async fn unregister_object(&self, guid: &str) {
        self.unregister_object_sync(guid);
    }

    fn unregister_object_sync(&self, guid: &str) {
        self.objects.lock().remove(guid);
    }

    async fn get_object(&self, guid: &str) -> Result<Arc<dyn ChannelOwner>> {
        self.objects.lock().get(guid).cloned().ok_or_else(|| {
            Error::ObjectNotFound(format!(
                "{} (object may have been closed or disposed)",
                guid
            ))
        })
    }

    async fn get_all_objects(&self) -> Vec<Arc<dyn ChannelOwner>> {
        self.objects.lock().values().cloned().collect()
    }

    fn all_objects_sync(&self) -> Vec<Arc<dyn ChannelOwner>> {
        self.objects.lock().values().cloned().collect()
    }

    fn selectors(&self) -> Arc<Selectors> {
        Arc::clone(&self.selectors)
    }

    fn close_writer(&self) -> bool {
        match self.sender.try_lock() {
            Ok(mut guard) => guard.take().is_some(),
            Err(_) => false,
        }
    }

    async fn wait_for_object(&self, guid: &str) -> Result<Arc<dyn ChannelOwner>> {
        Connection::wait_for_object(self, guid).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn response_parses_with_unexpected_error_details_shape() {
        // A future driver could change the `errorDetails` wire shape (here
        // `timedOut` arrives as a number, not a bool). The Response must still
        // deserialize: a parse failure drops the message in the run loop, so the
        // pending callback never resolves and the caller — which has no
        // client-side timeout — hangs forever.
        let raw = serde_json::json!({
            "id": "abcd@7",
            "error": { "error": { "message": "boom" } },
            "errorDetails": { "timedOut": 1 }
        });
        let msg: Message =
            serde_json::from_value(raw).expect("Response must parse despite an odd errorDetails");
        let Message::Response(response) = msg else {
            panic!("expected a Response");
        };
        assert_eq!(response.id, "abcd@7");
        assert!(response.error.is_some());
        assert!(response.error_details.is_some());
    }

    struct CapturingSender(tokio::sync::mpsc::UnboundedSender<Value>);

    impl TransportSender for CapturingSender {
        fn send(
            &mut self,
            message: Value,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
            let _ = self.0.send(message);
            Box::pin(async { Ok(()) })
        }
    }

    struct IdleReceiver;

    impl TransportReceiver for IdleReceiver {
        fn run(
            &mut self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
            Box::pin(std::future::pending())
        }
    }

    #[tokio::test]
    async fn requests_carry_one_prefix_and_a_rising_ordinal() {
        let (wire_tx, mut wire_rx) = tokio::sync::mpsc::unbounded_channel();
        let (_server_tx, message_rx) = mpsc::channel(1);
        let connection = Arc::new(Connection::new(
            CapturingSender(wire_tx),
            IdleReceiver,
            message_rx,
        ));
        for _ in 0..2 {
            let connection = Arc::clone(&connection);
            crate::task::spawn(async move {
                let _ = connection
                    .send_message(String::new(), "initialize".into(), Value::Null)
                    .await;
            });
        }

        let mut ids = Vec::new();
        for _ in 0..2 {
            let request = tokio::time::timeout(Duration::from_secs(5), wire_rx.recv())
                .await
                .expect("a request reaches the wire")
                .expect("the wire is open");
            ids.push(
                request["id"]
                    .as_str()
                    .expect("the id is a string")
                    .to_owned(),
            );
        }
        ids.sort();
        let prefix = &connection.call_id_prefix;
        assert_eq!(ids, [format!("{prefix}@0"), format!("{prefix}@1")]);
    }

    #[test]
    fn call_id_prefix_is_four_lowercase_letters() {
        let prefixes: Vec<String> = (0..16).map(|_| call_id_prefix()).collect();
        for prefix in &prefixes {
            assert_eq!(prefix.len(), 4, "{prefix:?}");
            assert!(prefix.bytes().all(|b| b.is_ascii_lowercase()), "{prefix:?}");
        }
        assert!(
            prefixes
                .iter()
                .any(|p| p.bytes().any(|b| b != p.as_bytes()[0])),
            "each letter draws on its own bits: {prefixes:?}"
        );
    }
}
