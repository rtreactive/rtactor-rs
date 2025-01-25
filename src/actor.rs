use super::active;
use super::reactive;
use std::any::Any;
use std::cmp;
use std::sync::Mutex;

////////////////////////////// public types /////////////////////////////////////

/// Way to send message to an actor.
#[derive(Debug, Clone)]
pub struct Addr {
    pub(crate) kind: AddrKind,
}

/// The basic block of information that can be exchanged between actors.
#[derive(Debug)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

/// Identifier of a request that allows to drop response not part of the transaction.
pub type RequestId = u32;

/// Message to ask something and that will generate a `Response` to `src` with `request_id`=`id`.
#[derive(Debug)]
pub struct Request {
    pub src: Addr,
    pub id: RequestId,
    pub data: Box<dyn Any + Send>,
}

/// Message to respond to a `Request`.
#[derive(Debug)]
pub struct Response {
    pub request_id: RequestId,
    pub result: Result<Box<dyn Any + Send>, ErrorStatus>,
}

/// Message without any confirmation.
#[derive(Debug)]
pub struct Notification {
    pub data: Box<dyn Any + Send>,
}

/// Error returned during actor operations.
// TODO  perhaps break into different enum depending on the context.
#[derive(Debug)]
pub enum Error {
    /// This address do not point to any actor.
    AddrUnreachable,
    /// The queue of the actor is full.
    QueueFull,
    /// The mechanism used to receive message had an error.
    BrokenReceive,
    /// The request was queued but the actor disappeared before the processing of the message.
    ActorDisappeared,
    /// An operation with a maximum fixed duration failed to complete in time.
    Timeout,
    /// A `try_wait_message` returned no message.
    NoMessage,
    /// An attempt to downcast a dyn any failed.
    DowncastFailed,
}

/// Error information returned in the Response when a Request failed at the transport level.
#[derive(Debug)]
pub struct ErrorStatus {
    pub error: Error,
    pub request_data: Box<dyn Any + Send>,
}

////////////////////////////// internal types /////////////////////////////////////

/// Identify a actor in an unique way.
///
/// This is necessary to compare actor addresses.
/// The memory address can not be used because an actor
/// could be recreated at the same memory space of a previous
/// actor.
pub(crate) type ActorId = u64;

pub(crate) const INVALID_ACTOR_ID: ActorId = 0;

static NEXT_ACTOR_ID: Mutex<ActorId> = Mutex::new(1);

pub(crate) fn generate_actor_id() -> ActorId {
    let mut next_actor_id = NEXT_ACTOR_ID.lock().unwrap();

    let out = *next_actor_id;
    *next_actor_id += 1;
    out
}

/// Different implementation of an actor.
#[derive(Debug, Clone)]
pub(crate) enum AddrKind {
    Invalid,
    Reactive(reactive::ReactiveAddr),
    Active(active::ActiveAddr),
}

/// Used to transport `ErrorStatus` before it is Boxed.
pub(crate) struct NonBoxedErrorStatus<T>
where
    T: Send + Sized,
{
    pub error: Error,
    pub request_data: T,
}

////////////////////////////// public fn's /////////////////////////////////////

/// Send notification.
///
/// ProcessContext::send_notification() and ActiveActor::send_notification()
/// are preferred because they allow future thread local memory allocation.
pub fn send_notification<T>(dst_addr: &Addr, data: T) -> Result<(), Error>
where
    T: 'static + Send,
{
    dst_addr.receive_notification(data)
}

////////////////////////////// public impl's /////////////////////////////////////
impl Message {
    /// Create a empty message of type Notification with empty ()) data.
    pub fn new() -> Message {
        Message::Notification(Notification { data: Box::new(()) })
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::new()
    }
}

impl Addr {
    pub const INVALID: Addr = Addr {
        kind: AddrKind::Invalid,
    };

    /// Create an new Invalid address.
    pub fn new() -> Addr {
        Addr {
            kind: AddrKind::Invalid,
        }
    }

    pub(crate) fn from_kind(kind: AddrKind) -> Self {
        Self { kind }
    }

    pub(crate) fn actor_id(&self) -> ActorId {
        match &self.kind {
            AddrKind::Invalid => INVALID_ACTOR_ID,
            AddrKind::Reactive(reactive_addr) => reactive_addr.actor_id(),
            AddrKind::Active(active_addr) => active_addr.actor_id(),
        }
    }

    /// Return true if the address points to a valid implementation.
    ///
    /// This does not imply the actor is reachable or still exists,
    /// only the address is not invalid.
    pub fn is_valid(&self) -> bool {
        !matches!(self.kind, AddrKind::Invalid)
    }

    /// Allows to send a notification to the address.
    pub(crate) fn receive_notification<T>(&self, data: T) -> Result<(), Error>
    where
        T: 'static + Send,
    {
        match &self.kind {
            AddrKind::Invalid => Result::Err(Error::AddrUnreachable),
            AddrKind::Reactive(reactive_addr) => reactive_addr.receive_notification(data),
            AddrKind::Active(active_addr) => active_addr.receive_notification(data),
        }
    }

    pub(crate) fn receive_request<T: 'static>(&self, src: &Addr, id: RequestId, data: T)
    where
        T: 'static + Send,
    {
        match &self.kind {
            AddrKind::Invalid => {
                let _ = src.receive_err_response(
                    id,
                    NonBoxedErrorStatus {
                        error: Error::AddrUnreachable,
                        request_data: data,
                    },
                );
            }

            AddrKind::Reactive(reactive_addr) => reactive_addr.receive_request(src, id, data),

            AddrKind::Active(active_addr) => active_addr.receive_request(src, id, data),
        }
    }

    pub(crate) fn receive_ok_response<T>(
        &self,
        request_id: RequestId,
        result: T,
    ) -> Result<(), Error>
    where
        T: 'static + Send + Sized,
    {
        match &self.kind {
            AddrKind::Invalid => Result::Err(Error::AddrUnreachable),
            AddrKind::Reactive(reactive_addr) => {
                reactive_addr.receive_ok_response(request_id, result)
            }
            AddrKind::Active(active_addr) => active_addr.receive_ok_response(request_id, result),
        }
    }

    pub(crate) fn receive_err_response<T>(
        &self,
        request_id: RequestId,
        result: NonBoxedErrorStatus<T>,
    ) -> Result<(), Error>
    where
        T: 'static + Send + Sized,
    {
        match &self.kind {
            AddrKind::Invalid => Result::Err(Error::AddrUnreachable),
            AddrKind::Reactive(reactive_addr) => {
                reactive_addr.receive_err_response(request_id, result)
            }
            AddrKind::Active(active_addr) => active_addr.receive_err_response(request_id, result),
        }
    }
}

impl Default for Addr {
    fn default() -> Self {
        Self::new()
    }
}

impl cmp::Ord for Addr {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.actor_id().cmp(&other.actor_id())
    }
}

impl cmp::PartialOrd for Addr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl cmp::PartialEq for Addr {
    fn eq(&self, other: &Self) -> bool {
        self.actor_id() == other.actor_id()
    }
}

impl cmp::Eq for Addr {}

impl Response {
    #[deprecated(note = "The name of this function is a typo, use id_eq() instead")]
    pub fn iq_eq(&self, id: RequestId) -> bool {
        self.id_eq(id)
    }

    /// Return true if the request id is equal to a value.
    pub fn id_eq(&self, id: RequestId) -> bool {
        self.request_id == id
    }
}
