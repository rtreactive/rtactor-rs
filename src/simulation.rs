//! Various tools to allow time simulation of actors.

#[cfg(feature = "mockall")]
pub use crate::reactive_mocker::ReactiveMocker;

use super::actor::{self, Message, RequestId};
use super::reactive::{self, ProcessContext};
use crate::dispatcher::Dispatcher;
use crate::mpsc_dispatcher::{self, MpscDispatcher};
use crate::{ActiveActor, Addr, Error};

use std::ops::ControlFlow;
use std::time;
use std::time::Duration;

////////////////////////////// public types /////////////////////////////////////

/// A dispatcher used to simulate time execution of reactive actors.
pub struct SimulationDispatcher {
    disp: MpscDispatcher,
    timeout_scheduler: reactive::TimeoutScheduler,
    instant_source: SimulatedInstantSource,
    request_id_counter_backup: RequestId,
}

////////////////////////////// internal types /////////////////////////////////////

// Implement reactive::InstantSource for the simulation.
struct SimulatedInstantSource {
    elapsed_duration: Duration,
    simulated_now: time::Instant,
}

////////////////////////////// public macros /////////////////////////////////////

/// Create a struct that allows a simulation access to an address and implement some or more SyncRequester or SyncNotifier
///
/// Example:
/// ```rs
/// define_syn_accessor(MySyncAccessorStructName, FirstSyncRequesterName, SecondSyncRequesterName, ...);
/// ```
///
/// It will have a function `new(&'a RefCell<SimulationDispatcher>, &Addr) -> MySyncAccessorStructName<'a>`
///
#[macro_export]
macro_rules! define_sim_sync_accessor{
    ($sync_accessor_name:ident, $($sync_trait:ty),+)
    =>
    {
        pub struct $sync_accessor_name<'a> {
            active_actor: ::rtactor::ActiveActor,
            target_addr: ::rtactor::Addr,
            disp: &'a std::cell::RefCell<::rtactor::simulation::SimulationDispatcher>,
        }

        impl<'a> $sync_accessor_name<'a> {
            pub fn new<'b>(disp: &'a std::cell::RefCell<::rtactor::simulation::SimulationDispatcher>, target_addr: &'b::rtactor::Addr) -> $sync_accessor_name<'a> {
                $sync_accessor_name {
                    active_actor: ::rtactor::ActiveActor::new(1),
                    target_addr: target_addr.clone(),
                    disp,
                }
            }
            pub fn target_addr(&self) -> &::rtactor::Addr {&self.target_addr}
        }

        impl<'a> ::rtactor::SyncAccessor for $sync_accessor_name<'a> {

            fn send_notification<T>(&mut self, data: T) -> Result<(), ::rtactor::Error>
            where
            T: 'static + Send,
            {
                let addr = self.target_addr.clone();
                self.active_actor.send_notification(&addr, data)
            }
            fn request_for<TRequest, TResponse>(
                &mut self,
                request_data: TRequest,
                timeout: std::time::Duration,
            ) -> Result<TResponse, ::rtactor::Error>
            where
                TRequest: 'static + Send + Sized,
                TResponse: 'static + Send + Sized
            {
                let addr = self.target_addr.clone();
                self.disp.borrow_mut().active_request_for(&mut self.active_actor, &addr, request_data, timeout)
            }

        }

        $(
            impl<'a> $sync_trait for $sync_accessor_name<'a> {}
        )*
    }
}

////////////////////////////// public impl's /////////////////////////////////////

impl SimulationDispatcher {
    pub fn new(queue_size: usize) -> SimulationDispatcher {
        let disp = mpsc_dispatcher::Builder::new(queue_size).build();
        SimulationDispatcher {
            disp,
            request_id_counter_backup: 0,
            instant_source: SimulatedInstantSource::new(),
            timeout_scheduler: reactive::TimeoutScheduler::new(),
        }
    }

    pub fn addr(&self) -> Addr {
        self.disp.addr()
    }

    #[deprecated = "use the better named addr()"]
    pub fn get_addr(&self) -> Addr {
        self.addr()
    }

    /// Move reactive into the dispatcher.
    pub fn register_reactive(&mut self, behaviour: Box<dyn reactive::Behavior>) -> actor::Addr {
        self.disp.register_reactive(behaviour)
    }

    /// Replace reactive in the dispatcher.
    pub fn replace_reactive(
        &mut self,
        addr: &Addr,
        behaviour: Box<dyn reactive::Behavior>,
    ) -> Result<Box<dyn reactive::Behavior>, Box<dyn reactive::Behavior>> {
        self.disp.replace_reactive(addr, behaviour)
    }

    /// Get the current simulated time.
    pub fn now(&self) -> reactive::Instant {
        self.instant_source.now()
    }

    /// Process messages until either the `duration` is elapsed or `break_func` return `ControlFlow::Break`.
    pub fn process_for_cond<F>(&mut self, duration: Duration, break_func: F)
    where
        F: FnMut() -> ControlFlow<()>,
    {
        self.process_until_cond(self.now() + duration, break_func);
    }
    /*
        fn wait_active_message(&mut self, active_actor: &mut ActiveActor, timeout: Duration) -> Result<Message, actor::Error>
        {

        }
    */

    /// Process messages until either the `instant` is reached or `break_func` return `ControlFlow::Break`.
    pub fn process_until_cond<F>(&mut self, instant: reactive::Instant, mut break_func: F)
    where
        F: FnMut() -> ControlFlow<()>,
    {
        loop {
            let mut message_processed: bool;
            let mut stop: bool;
            loop {
                let mut context = ProcessContext::new(
                    &self.disp,
                    self.request_id_counter_backup,
                    &self.instant_source,
                    &mut self.timeout_scheduler,
                );

                // process all queued messages
                loop {
                    (message_processed, stop) = self.disp.try_process_message(&mut context);

                    match (break_func)() {
                        ControlFlow::Break(_) => {
                            stop = true;
                        }
                        ControlFlow::Continue(_) => {
                            let now = context.now();
                            if now > instant || (!message_processed && now == instant) {
                                stop = true;
                            }
                        }
                    }

                    if stop || !message_processed {
                        break;
                    }
                }
                self.request_id_counter_backup = context.request_id_counter();

                if stop {
                    break;
                } else {
                    // try to queue mature timeouts and continue to process them
                    match context.try_send_next_pending_timeout() {
                        ControlFlow::Continue(()) => (),
                        ControlFlow::Break(duration) => {
                            if duration != Duration::MAX && ((self.now() + duration) < instant) {
                                // If there is a timeout before instant, avance to it deadline.
                                self.instant_source.advance_for(duration);
                            } else if instant.at_inf() {
                                // If the instant is infinity, continue the processing, break_func can have some side effect.
                            } else {
                                // else advance the time at instant.
                                self.instant_source.advance_until(instant);
                            }
                        }
                    }
                }
            }

            if stop {
                break;
            }

            // advance until new timeout or timepoint reached, break if duration reached
            // min(instant, first_timeout_instant)
        }
    }

    /// Process messages until the `duration` is elapsed.
    pub fn process_for(&mut self, duration: Duration) {
        self.process_until(self.now() + duration);
    }

    /// Process messages until the `instant` is reached.
    pub fn process_until(&mut self, instant: reactive::Instant) {
        self.process_until_cond(instant, || ControlFlow::Continue(()))
    }

    /// Process messages until the dispatcher is stopped.
    pub fn process(&mut self) {
        self.process_until(reactive::Instant::INFINITY);
    }

    /// Wait indefinitely that an ActiveActor receives a message.
    pub fn active_wait_message(&mut self, active: &mut ActiveActor) -> Result<Message, Error> {
        self.active_wait_message_until(active, reactive::Instant::INFINITY)
    }

    /// Wait for a given duration that an ActiveActor receives a message.
    pub fn active_wait_message_for(
        &mut self,
        active: &mut ActiveActor,
        duration: Duration,
    ) -> Result<Message, Error> {
        self.active_wait_message_until(active, self.now() + duration)
    }

    /// Wait until a point in time that an ActiveActor receives a message.
    pub fn active_wait_message_until(
        &mut self,
        active: &mut ActiveActor,
        instant: reactive::Instant,
    ) -> Result<Message, Error> {
        self.active_wait_message_until_cond(active, instant, || ControlFlow::Continue(()))
    }

    /// Wait until a point in time that an ActiveActor receives a message or a condition is met.
    pub fn active_wait_message_until_cond<F>(
        &mut self,
        active: &mut ActiveActor,
        instant: reactive::Instant,
        mut break_func: F,
    ) -> Result<Message, Error>
    where
        F: FnMut() -> ControlFlow<()>,
    {
        let mut msg: actor::Message = actor::Message::new();
        let mut msg_received = false;
        let mut err: Error = Error::NoMessage;
        self.process_until_cond(instant, || match active.try_get_message() {
            Ok(receive_msg) => {
                msg_received = true;
                msg = receive_msg;
                ControlFlow::Break(())
            }
            Err(actor::Error::NoMessage) => (break_func)(),
            Err(e) => {
                msg_received = false;
                err = e;
                ControlFlow::Break(())
            }
        });

        if msg_received {
            Ok(msg)
        } else {
            Err(err)
        }
    }

    /// Execute a request with a `ActiveActor` until either the response arrives or `instant` is reached.
    pub fn active_request_until<TRequest, TResponse>(
        &mut self,
        active: &mut ActiveActor,
        dst: &actor::Addr,
        request_data: TRequest,
        instant: reactive::Instant,
    ) -> Result<TResponse, actor::Error>
    where
        TRequest: 'static + Send + Sized,
        TResponse: 'static + Send + Sized,
    {
        self.active_request_until_cond(active, dst, request_data, instant, || {
            ControlFlow::Continue(())
        })
    }

    /// Execute a request with a `ActiveActor` until either the response arrives, `break_func` breaks
    /// or `instant` is reached.
    pub fn active_request_until_cond<TRequest, TResponse, F>(
        &mut self,
        active: &mut ActiveActor,
        dst: &actor::Addr,
        request_data: TRequest,
        instant: reactive::Instant,
        mut break_func: F,
    ) -> Result<TResponse, actor::Error>
    where
        TRequest: 'static + Send + Sized,
        TResponse: 'static + Send + Sized,
        F: FnMut() -> ControlFlow<()>,
    {
        let request_id = active.generate_request_id();
        dst.receive_request(&active.addr(), request_id, request_data);

        let mut msg: actor::Message = actor::Message::new();
        let mut msg_received = false;
        let mut err = actor::Error::NoMessage;
        self.process_until_cond(instant, || match active.try_get_message() {
            Ok(receive_msg) => {
                msg = receive_msg;
                msg_received = true;
                ControlFlow::Break(())
            }
            Err(actor::Error::NoMessage) => (break_func)(),
            Err(e) => {
                err = e;
                ControlFlow::Break(())
            }
        });

        if msg_received {
            match msg {
                actor::Message::Response(response) => {
                    if response.id_eq(request_id) {
                        match response.result {
                            Ok(data) => match data.downcast::<TResponse>() {
                                Ok(out) => Ok(*out),
                                Err(_) => Err(actor::Error::DowncastFailed),
                            },
                            Err(err) => Err(err.error),
                        }
                    } else {
                        Err(actor::Error::NoMessage)
                    }
                }
                _ => Err(actor::Error::NoMessage),
            }
        } else {
            Err(err)
        }
    }

    /// Execute a request with a `ActiveActor` until either the response arrives
    /// or `duration` elapses.
    pub fn active_request_for<TRequest, TResponse>(
        &mut self,
        active: &mut ActiveActor,
        dst: &actor::Addr,
        request_data: TRequest,
        duration: Duration,
    ) -> Result<TResponse, actor::Error>
    where
        TRequest: 'static + Send + Sized,
        TResponse: 'static + Send + Sized,
    {
        self.active_request_until(active, dst, request_data, self.now() + duration)
    }
}

////////////////////////////// internal impl's /////////////////////////////////////

impl SimulatedInstantSource {
    pub fn new() -> SimulatedInstantSource {
        SimulatedInstantSource {
            elapsed_duration: Duration::from_secs(0),
            simulated_now: time::Instant::now(),
        }
    }

    pub fn advance_for(&mut self, duration: Duration) {
        self.elapsed_duration += duration;
        self.simulated_now += duration;
    }

    pub fn advance_until(&mut self, instant: reactive::Instant) {
        match instant.internal() {
            reactive::InternalInstant::Finite(internal_instant) => {
                self.elapsed_duration += *internal_instant - self.simulated_now;
                self.simulated_now = *internal_instant;
            }
            reactive::InternalInstant::Infinity => panic!(),
        }
    }

    pub fn now(&self) -> reactive::Instant {
        reactive::Instant::new(self.simulated_now)
    }
}

impl reactive::InstantSource for SimulatedInstantSource {
    fn now(&self) -> reactive::Instant {
        self.now()
    }
}
