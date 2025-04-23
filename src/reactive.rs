//! Type of actors where messages handled by a dispatcher and that call the actor to process them.

// TODO use lock-free mpsc
// see https://docs.rs/crossbeam/0.3.2/crossbeam/index.html

// TODO use lock-free pool:
// https://docs.rs/heapless/0.5.5/heapless/pool/index.html

use crate::Notification;

use crate::actor;
use crate::actor::AddrKind;
use crate::actor::{ActorId, Message, RequestId};
use crate::mpsc_dispatcher::MpscDispatcher;

use std::boxed::Box;

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Add;
use std::ops::ControlFlow;
use std::sync::mpsc;

use std::time;
use std::time::Duration;

////////////////////////////// public types /////////////////////////////////////

/// The custom implementation of message handling for a given reactive actor.
///
/// There are two ways to write mocks of reactive actors. There is the very simple
/// `MockBehavior` that mocks directly this trait. Or a more complex and powerful
/// `simulation::ReactiveMocker` that allows to change the mock during the tests, user
/// data, etc.
#[cfg_attr(feature = "mockall", mockall::automock)]
pub trait Behavior {
    #[allow(clippy::needless_lifetimes)] // false positive (tested with 1.64.0 and 1.86.0)
    fn process_message<'a>(&mut self, context: &mut ProcessContext<'a>, msg: &Message);
}

/// An way to interact with the dispatcher that is only valid during `Behavior::process_message()`.
pub struct ProcessContext<'a> {
    tx: mpsc::SyncSender<MessageAndDstId>,
    pub(crate) own_actor_id: actor::ActorId,
    request_id_counter: RequestId,
    instant_source: &'a dyn InstantSource,
    timeout_scheduler: &'a mut TimeoutScheduler,
}

/// Point in time similar to `std::time::Instant` but that allows simulation for reactive.
#[derive(Debug, Clone, Copy)]
pub struct Instant(InternalInstant);

/// Data send in Notification when a timer mature.
///
/// Use `timer.is_scheduling(notif)` to check if a timer is ready.
#[derive(Debug)]
pub struct Timeout(TimeoutId);

/// An handle to a timer that can send a message to a Behavior at a given time.
///
/// Timers can be freely created and destroyed. It is best to call `context.unschedule(timer)`
/// on them before destruction to avoid to retain an entry in the timer scheduling mechanism.
/// If not the timer will be dispatched but never trigger any is_scheduling() == true code.
#[derive(Clone, Copy)]
pub struct Timer {
    id: TimeoutId,
    deadline: Instant,
}

/// An implementation of behavior that do nothing in process_message().
///
/// This can be useful to create circular address dependencies at build time:
/// ```
/// # use rtactor as rt;
/// # struct Sink();
/// # impl Sink {pub fn new(_: &rt::Addr) -> Self {Self()}}
/// # impl rt::Behavior for Sink {fn process_message<'a, 'b>(&mut self, _context: &'a mut rt::ProcessContext<'b>, _msg: &rt::Message) {}}
/// # struct Source();
/// # impl Source {pub fn new(_: &rt::Addr) -> Self {Self()}}
/// # impl rt::Behavior for Source {fn process_message<'a, 'b>(&mut self, _context: &'a mut rt::ProcessContext<'b>, _msg: &rt::Message) {}}
/// #
/// # let builder = rt::mpsc_dispatcher::Builder::new(1);
/// # let mut disp_accessor = builder.to_accessor();
/// # std::thread::spawn(move || builder.build().process());
///
/// let sink_addr = disp_accessor.register_reactive_unwrap(rt::DummyBehavior::default());
/// let source_addr = disp_accessor.register_reactive_unwrap(Source::new(&sink_addr));
/// disp_accessor.replace_reactive_unwrap(&sink_addr, Sink::new(&source_addr));
/// ```

#[derive(Default)]
pub struct DummyBehavior();

impl Behavior for DummyBehavior {
    fn process_message(&mut self, _context: &mut ProcessContext, _msg: &Message) {}
}

////////////////////////////// internal types /////////////////////////////////////

/// An address pointing to an actor that "reacts" (is called) when a message is received.
#[derive(Debug, Clone)]
pub(crate) struct ReactiveAddr {
    tx: mpsc::SyncSender<MessageAndDstId>,
    pub dst_id: actor::ActorId,
}

/// What is stored in the queue.
pub(crate) struct MessageAndDstId {
    pub message: Message,
    pub dst_id: actor::ActorId,
}

/// Private implementation of a `reactive::Instant`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum InternalInstant {
    Finite(time::Instant),
    Infinity,
}

/// Source of `reactive::Instant`, for example real or simulated.
pub(crate) trait InstantSource {
    fn now(&self) -> Instant;
}

/// Identify a timeout to avoid flying notification of unscheduled timer to trigger code.
type TimeoutId = u32;

/// Entity responsible to manage timer registrations and timeouts.
pub(crate) struct TimeoutScheduler {
    next_timeout_id: TimeoutId,
    timeout_list: BTreeMap<Timer, ActorId>,
}

////////////////////////////// public impl's /////////////////////////////////////

impl<'a> ProcessContext<'a> {
    pub(crate) fn new(
        disp: &MpscDispatcher,
        request_id_counter: RequestId,
        instant_source: &'a dyn InstantSource,
        timeout_scheduler: &'a mut TimeoutScheduler,
    ) -> ProcessContext<'a> {
        ProcessContext {
            tx: disp.tx.clone(),
            own_actor_id: actor::INVALID_ACTOR_ID,
            request_id_counter,
            instant_source,
            timeout_scheduler,
        }
    }

    /// Return the actor address of the reactive being processed.
    pub fn own_addr(&self) -> actor::Addr {
        ReactiveAddr::new(self.tx.clone(), self.own_actor_id).into_addr()
    }

    #[deprecated = "use own_addr()"]
    pub fn get_own_addr(&self) -> actor::Addr {
        self.own_addr()
    }

    /// Send a request to an other actor.
    ///
    /// Return an identifier of the transaction.
    pub fn send_request<T>(&mut self, dst: &actor::Addr, data: T) -> RequestId
    where
        T: 'static + Send,
    {
        self.request_id_counter = self.request_id_counter.wrapping_add(1);
        dst.receive_request(&self.own_addr(), self.request_id_counter, data);
        self.request_id_counter
    }

    /// Send the response to a request.
    pub fn send_response<T>(&mut self, request: &actor::Request, data: T)
    where
        T: 'static + Send,
    {
        self.send_addr_id_response(&request.src, request.id, data);
    }

    /// Send the response to a request using destination and request id.
    pub fn send_addr_id_response<T>(&mut self, dst: &actor::Addr, request_id: RequestId, data: T)
    where
        T: 'static + Send,
    {
        // TODO should we retry ? wait ? block ?
        let _result = dst.receive_ok_response(request_id, data);
    }

    pub fn send_self_notification<T>(&mut self, data: T) -> Result<(), crate::Error>
    where
        T: 'static + Send,
    {
        // There is room here for optimization.
        self.own_addr().receive_notification(data)
    }

    /// Send notification.
    ///
    /// This method is equivalent to the function rtactor::send_notification()
    /// but is preferred because it allows possible future thread local memory allocation.
    pub fn send_notification<T>(&mut self, dst: &actor::Addr, data: T) -> Result<(), crate::Error>
    where
        T: 'static + Send,
    {
        dst.receive_notification(data)
    }

    pub fn schedule_until(&mut self, timer: &mut Timer, instant: Instant) {
        self.timeout_scheduler
            .schedule_until(timer, instant, self.own_actor_id);
    }

    /// Schedule a timer for a given duration.
    pub fn schedule_for(&mut self, timer: &mut Timer, duration: Duration) {
        // insert timer
        self.schedule_until(timer, self.instant_source.now() + duration);
    }

    /// Unschedule a timer.
    pub fn unschedule(&mut self, timer: &mut Timer) {
        self.timeout_scheduler.unschedule(timer, self.own_actor_id);
        // find in the list from the most recent and remove
    }

    /// Return the current time.
    pub fn now(&self) -> Instant {
        self.instant_source.now()
    }

    pub(crate) fn request_id_counter(&self) -> RequestId {
        self.request_id_counter
    }

    /// If one timer is mature send a notification for it.
    ///
    /// Return ControlFlow::Continue(()) if a timeout was sent. If not,
    /// return ControlFlow::Break(duration) with the waiting time to the next
    /// timeout or Duration::MAX if there is no timeout.
    pub(crate) fn try_send_next_pending_timeout(&mut self) -> ControlFlow<Duration, ()> {
        match self.timeout_scheduler.timeout_list.iter().next() {
            Some((key, reactive_id)) => {
                let duration_to_next = key.deadline.saturating_sub(&self.now());
                // if timeout is ready
                if duration_to_next == Duration::ZERO {
                    {
                        let key_copy = *key;
                        let reactive_id_copy = *reactive_id;
                        self.timeout_scheduler
                            .timeout_list
                            .remove(&key_copy)
                            .unwrap();

                        // try to send the notification
                        match self.tx.try_send(MessageAndDstId {
                            message: Message::Notification(Notification {
                                data: Box::new(Timeout(key_copy.id)),
                            }),
                            dst_id: reactive_id_copy,
                        }) {
                            // if send ok, continue to process messages
                            Ok(_) => ControlFlow::Continue(()),
                            // if send failed, store back the timeout and let the processing
                            // of message solve the problem (detect queue disconnect or make space if full)
                            Err(_) => {
                                self.timeout_scheduler
                                    .timeout_list
                                    .insert(key_copy, reactive_id_copy);
                                ControlFlow::Continue(())
                            }
                        }
                    }
                } else {
                    // If timeout not ready, break to block on the queue for the needed duration.
                    ControlFlow::Break(duration_to_next)
                }
            }
            // If no timeout, break to wait indefinitely new messages on the queue
            None => ControlFlow::Break(Duration::MAX),
        }
    }
}

impl Instant {
    pub const INFINITY: Instant = Instant(InternalInstant::Infinity);

    /// Return true if the instant represent infinity.
    ///
    /// This is the case for `Instant::INFINITY` or `now() + Duration::MAX`.
    pub fn at_inf(&self) -> bool {
        match self.0 {
            InternalInstant::Finite(_) => false,
            InternalInstant::Infinity => true,
        }
    }

    /// Set the instant to Instant::INFINITY.
    pub fn set_inf(&mut self) {
        self.0 = InternalInstant::Infinity;
    }

    pub fn new(instant: time::Instant) -> Instant {
        Instant(InternalInstant::Finite(instant))
    }

    pub(crate) fn internal(&self) -> &InternalInstant {
        &self.0
    }

    /// Subtract two instants and saturate to Duration::ZERO.
    pub fn saturating_sub(&self, rhs: &Instant) -> Duration {
        match self.internal() {
            InternalInstant::Finite(self_internal) => match rhs.internal() {
                InternalInstant::Finite(rhs_internal) => {
                    if self_internal < rhs_internal {
                        Duration::ZERO
                    } else {
                        self_internal.duration_since(*rhs_internal)
                    }
                }
                InternalInstant::Infinity => Duration::ZERO,
            },
            InternalInstant::Infinity => match rhs.internal() {
                InternalInstant::Finite(_) => Duration::MAX,
                InternalInstant::Infinity => panic!(),
            },
        }
    }
}

#[cfg(test)]
mod instant_tests {
    use super::*;

    #[test]
    fn test_instant_is_inf() {
        let begin = Instant::new(std::time::Instant::now());

        assert!(!begin.at_inf());
        assert!((begin + Duration::MAX).at_inf());
        assert!(Instant::INFINITY.at_inf());
    }

    #[test]
    fn test_instant_sub() {
        let begin = std::time::Instant::now();

        let a = Instant::new(begin);
        let b = a + Duration::from_secs(13);

        assert!(b.saturating_sub(&a) == Duration::from_secs(13));
        assert!(a.saturating_sub(&b) == Duration::ZERO);

        assert!(Instant::INFINITY.saturating_sub(&a) == Duration::MAX);
        assert!(b.saturating_sub(&Instant::INFINITY) == Duration::ZERO);
    }

    #[cfg(test)]
    #[test]
    #[should_panic]
    fn test_instant_inf_sub_inf() {
        Instant::INFINITY.saturating_sub(&Instant::INFINITY);
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, other: Duration) -> Instant {
        if other == Duration::MAX {
            Instant(InternalInstant::Infinity)
        } else {
            match self.0 {
                InternalInstant::Finite(internal) => {
                    Instant(InternalInstant::Finite(internal + other))
                }
                InternalInstant::Infinity => Instant(InternalInstant::Infinity),
            }
        }
    }
}

impl PartialEq<Instant> for Instant {
    fn eq(&self, other: &Instant) -> bool {
        match self.0 {
            InternalInstant::Finite(internal) => match other.0 {
                InternalInstant::Finite(other_internal) => internal == other_internal,
                InternalInstant::Infinity => false,
            },
            InternalInstant::Infinity => match other.0 {
                InternalInstant::Finite(_) => false,
                InternalInstant::Infinity => true,
            },
        }
    }
}

impl PartialOrd<Instant> for Instant {
    fn partial_cmp(&self, other: &Instant) -> Option<Ordering> {
        match self.0 {
            InternalInstant::Finite(internal) => match other.0 {
                InternalInstant::Finite(other_internal) => internal.partial_cmp(&other_internal),
                InternalInstant::Infinity => Some(Ordering::Less),
            },
            InternalInstant::Infinity => match other.0 {
                InternalInstant::Finite(_) => Some(Ordering::Greater),
                InternalInstant::Infinity => Some(Ordering::Equal),
            },
        }
    }
}

////////////////////////////// internal impl's /////////////////////////////////////

impl InternalInstant {
    pub(crate) fn into_instant(self) -> crate::Instant {
        Instant(self)
    }
}

impl ReactiveAddr {
    pub(crate) fn new(tx: mpsc::SyncSender<MessageAndDstId>, dst_id: actor::ActorId) -> Self {
        Self { tx, dst_id }
    }

    pub(crate) fn into_addr(self) -> actor::Addr {
        actor::Addr::from_kind(AddrKind::Reactive(self))
    }
    pub(crate) fn actor_id(&self) -> actor::ActorId {
        self.dst_id
    }
    pub(crate) fn receive_notification<T>(&self, data: T) -> Result<(), actor::Error>
    where
        T: 'static + Send + Sized,
    {
        let message = MessageAndDstId {
            dst_id: self.dst_id,
            message: Message::Notification(actor::Notification {
                data: Box::new(data),
            }),
        };

        match self.tx.try_send(message) {
            Ok(_) => Result::Ok(()),
            Err(err) => Result::Err(match err {
                mpsc::TrySendError::Full(_) => actor::Error::QueueFull,
                mpsc::TrySendError::Disconnected(..) => actor::Error::AddrUnreachable,
            }),
        }
    }

    pub fn receive_request<T>(&self, src: &actor::Addr, id: actor::RequestId, data: T)
    where
        T: 'static + Send + Sized,
    {
        let message = MessageAndDstId {
            dst_id: self.dst_id,
            message: Message::Request(actor::Request {
                src: src.clone(),
                id,
                data: Box::new(data),
            }),
        };

        if let Err(err) = self.tx.try_send(message) {
            let (actor_err, returned_data) = match err {
                mpsc::TrySendError::Full(a_data) => (actor::Error::QueueFull, a_data),
                mpsc::TrySendError::Disconnected(a_data) => (actor::Error::AddrUnreachable, a_data),
            };
            // try to send an error response to himself
            let _ = src.receive_err_response(
                id,
                actor::NonBoxedErrorStatus {
                    error: actor_err,
                    request_data: returned_data.message,
                },
            );
        }
    }

    pub(super) fn receive_ok_response<T>(
        &self,
        request_id: RequestId,
        result: T,
    ) -> Result<(), actor::Error>
    where
        T: 'static + Send + Sized,
    {
        let response = actor::Response {
            request_id,
            result: Result::Ok(Box::new(result)),
        };

        let message_and_dst_id = MessageAndDstId {
            dst_id: self.dst_id,
            message: actor::Message::Response(response),
        };

        match self.tx.try_send(message_and_dst_id) {
            Ok(_) => Result::Ok(()),
            Err(err) => Result::Err(match err {
                mpsc::TrySendError::Full(_) => actor::Error::QueueFull,
                mpsc::TrySendError::Disconnected(..) => actor::Error::AddrUnreachable,
            }),
        }
    }

    pub(super) fn receive_err_response<T>(
        &self,
        request_id: RequestId,
        result: actor::NonBoxedErrorStatus<T>,
    ) -> Result<(), actor::Error>
    where
        T: 'static + Send + Sized,
    {
        let boxed_result = actor::ErrorStatus {
            error: result.error,
            request_data: Box::new(result.request_data),
        };

        let response = actor::Response {
            request_id,
            result: Result::Err(boxed_result),
        };

        let message_and_dst_id = MessageAndDstId {
            dst_id: self.dst_id,
            message: actor::Message::Response(response),
        };

        match self.tx.try_send(message_and_dst_id) {
            Ok(_) => Result::Ok(()),
            Err(err) => Result::Err(match err {
                mpsc::TrySendError::Full(_) => actor::Error::QueueFull,
                mpsc::TrySendError::Disconnected(..) => actor::Error::AddrUnreachable,
            }),
        }
    }
}

impl TimeoutScheduler {
    pub(crate) fn new() -> TimeoutScheduler {
        TimeoutScheduler {
            next_timeout_id: 0,
            timeout_list: BTreeMap::new(),
        }
    }

    fn generate_timeout_id(&mut self) -> TimeoutId {
        let out = self.next_timeout_id;
        self.next_timeout_id = self.next_timeout_id.wrapping_add(1);
        out
    }

    /// Schedule a timer at a point in the future.
    pub(crate) fn schedule_until(
        &mut self,
        timer: &mut Timer,
        instant: Instant,
        dst: actor::ActorId,
    ) {
        self.unschedule(timer, dst);

        timer.id = self.generate_timeout_id();
        timer.deadline = instant;
        self.timeout_list.insert(*timer, dst);
        assert!(!self.timeout_list.is_empty());
    }

    /// Remove the timer to the scheduled list if present.
    pub(crate) fn unschedule(&mut self, timer: &mut Timer, dst: actor::ActorId) {
        if !timer.deadline.at_inf() {
            match self.timeout_list.entry(*timer) {
                std::collections::btree_map::Entry::Vacant(_) => (),
                std::collections::btree_map::Entry::Occupied(occupied_entry) => {
                    if *occupied_entry.get() == dst {
                        occupied_entry.remove();
                    }
                }
            }
            timer.deadline.set_inf();
        }
    }
}

impl Timer {
    /// Create an unscheduled timer.
    pub fn new() -> Timer {
        Timer {
            id: 0,
            deadline: Instant::INFINITY,
        }
    }

    /// Check that a notification was created to schedule this timer timeout event.
    ///
    /// The method must always be used to check if the code that the timer should trigger
    /// needs to be executed. It takes care of race conditions like timer unscheduled after
    /// the timeout notification was send to the queue.
    pub fn is_scheduling(&mut self, notif: &Notification) -> bool {
        match notif.data.downcast_ref::<Timeout>() {
            Some(timeout) => {
                if timeout.0 == self.id {
                    self.deadline.set_inf();
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }
}

impl Default for Timer {
    fn default() -> Self {
        Timer::new()
    }
}

impl std::cmp::Ord for Timer {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.deadline.partial_cmp(&other.deadline) {
            Some(Ordering::Greater) => Ordering::Greater,
            Some(Ordering::Less) => Ordering::Less,
            _ => self.id.cmp(&other.id),
        }
    }
}

impl std::cmp::PartialOrd for Timer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::cmp::Eq for Timer {}

impl std::cmp::PartialEq for Timer {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.deadline == other.deadline
    }
}

////////////////////////////// tests /////////////////////////////////////

#[cfg(test)]
mod tests {

    use std::mem;

    use super::*;

    #[test]
    fn size_of_objects() {
        assert!(mem::size_of::<mpsc::SyncSender<Message>>() <= 16);
        assert!(mem::size_of::<Message>() <= 56);
        assert!(mem::size_of::<actor::Request>() <= 56);
        assert!(mem::size_of::<actor::Response>() <= 40);
        assert!(mem::size_of::<actor::Notification>() <= 16);
        assert!(mem::size_of::<actor::Addr>() <= 32);
        assert!(mem::size_of::<ReactiveAddr>() <= 24);
    }

    #[test]
    fn test_timer_compare() {
        let begin = Instant::new(time::Instant::now());
        let mut end;
        loop {
            end = Instant::new(time::Instant::now());
            if end.saturating_sub(&begin) > Duration::ZERO {
                break;
            }
        }

        assert!(
            Timer {
                id: 345,
                deadline: begin
            } == Timer {
                id: 345,
                deadline: begin
            }
        );
        assert!(
            Timer {
                id: 1,
                deadline: begin
            } > Timer {
                id: 0,
                deadline: begin
            }
        );

        assert!(
            Timer {
                id: 13,
                deadline: begin
            } < Timer {
                id: 45,
                deadline: end
            }
        );
        assert!(
            Timer {
                id: 13,
                deadline: end
            } > Timer {
                id: 45,
                deadline: begin
            }
        );

        assert!(
            Timer {
                id: 0,
                deadline: InternalInstant::Infinity.into_instant()
            } > Timer {
                id: u32::MAX,
                deadline: begin
            }
        );

        assert!(
            Timer {
                id: 1234567,
                deadline: InternalInstant::Infinity.into_instant()
            } < Timer {
                id: 1234568,
                deadline: InternalInstant::Infinity.into_instant()
            }
        );
        assert!(
            Timer {
                id: 55555,
                deadline: InternalInstant::Infinity.into_instant()
            } == Timer {
                id: 55555,
                deadline: InternalInstant::Infinity.into_instant()
            }
        );
    }
}
