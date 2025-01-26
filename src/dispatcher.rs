//! An actor interface to a dispatcher for reactive actor.

use rtactor_macros::{ResponseEnum, SyncRequester};
use std::boxed::Box;
use std::time::Duration;

use crate::ActiveActor;

use super::actor::Addr;
use super::reactive::Behavior;
use std::any::Any;
use std::cell::Cell;

pub trait Dispatcher {
    #[deprecated = "use the better named addr()"]
    fn own_addr(&self) -> Addr {
        self.addr()
    }

    /// Return the actor address of the dispatcher itself.
    fn addr(&self) -> Addr;

    /// Migrate a reactive inside the dispatcher and return an actor address to it.
    fn register_reactive(&mut self, behavior: Box<dyn Behavior>) -> Addr;

    /// Replace an implementation of a reactive by a new one.
    /// This only change the object that responds to process message.
    /// This is thread safe but of course the management of new and old
    /// inner states transition should be taken into account.
    /// All copies of the address are still valid.
    ///
    /// Return Ok(old_behavior) or Err(new_behavior)
    fn replace_reactive(
        &mut self,
        addr: &Addr,
        behavior: Box<dyn Behavior>,
    ) -> Result<Box<dyn Behavior>, Box<dyn Behavior>>;

    /// Extract a reactive from the dispatcher by it address.
    fn unregister_reactive(&mut self, addr: &Addr) -> Option<Box<dyn Behavior>>;
}

/// A function that can be executed by the dispatcher with a `dispatcher::Request::ExecuteFn`.
pub type ExecutableFn = dyn FnOnce(&mut dyn Dispatcher) -> Box<dyn Any + Send> + Send;

#[derive(ResponseEnum, SyncRequester)]
pub enum Request {
    /// Register a reactive that is Send.
    /// To register reactives that are not Send, do it in the spawn_dispatcher() function.
    #[response_val(Addr)]
    RegisterReactive {
        behavior: Cell<Option<Box<dyn Behavior + Send>>>,
    },

    // TODO deprecated because it make no sense. "#[deprecated]" is not used because it create a
    // warning probably due to the ResponseEnum proc_macro
    #[response_val(Box<dyn Behavior + Send>)]
    StopReactive { addr: Addr },

    /// Stop the dispatch of message and return from process().
    ///
    /// All Request not processed are responded with a Error::ActorDisappeared. Then
    /// all registered behavior are destroyed. Finally a Response::StopDispatcher is sent.
    #[response_val()]
    StopDispatcher {},

    /// Execute a FnOnce inside the dispatcher thread.
    /// The Response returns the return value of the FnOnce.
    /// The Cell is necessary to allow FnOnce in the implementation.
    #[response_val(Box<dyn Any + Send>)]
    ExecuteFn {
        executable_fn: Cell<Box<ExecutableFn>>,
    },
}

/// An accessor for a dispatcher with specialized functions.
pub struct SyncAccessor {
    active_actor: ActiveActor,
    disp_addr: Addr,
}

impl SyncAccessor {
    /// Create an accessor from a dispatcher address.
    pub fn new(disp_addr: &Addr) -> Self {
        Self {
            active_actor: ActiveActor::new(1),
            disp_addr: disp_addr.clone(),
        }
    }

    /// Build from an existing dispatcher.
    #[deprecated = "use new() instead and do join() manually if needed"]
    pub fn from_existing_dispatcher(
        disp_addr: Addr,
        _join_handle: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            active_actor: ActiveActor::new(1),
            disp_addr,
        }
    }

    /// Return the dispatcher address.
    pub fn dispatcher_addr(&self) -> &Addr {
        &self.disp_addr
    }

    /// Register a reactive that will be moved in a Box into the dispatcher.
    ///
    /// If the Behavior is not Send, the pattern to use is to use execute_fn()
    /// and to construct the behavior implementation in f.
    pub fn register_reactive_for<T>(
        &mut self,
        reactive: T,
        timeout: Duration,
    ) -> Result<Addr, crate::Error>
    where
        T: Behavior + Send + 'static,
    {
        let result = self.active_actor.request_for::<_, Response>(
            &self.disp_addr,
            Request::RegisterReactive {
                behavior: Cell::new(Some(Box::new(reactive))),
            },
            timeout,
        );

        match result {
            Ok(response) => {
                if let Response::RegisterReactive(addr) = response {
                    Ok(addr)
                } else {
                    Err(crate::Error::DowncastFailed)
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Replace a reactive and drop the older one in the dispatcher thread.
    ///
    /// Because Behavior are not Send in the dispatcher, ones as to use execute_fn()
    /// replace_reactive() to get the old behavior.
    pub fn replace_reactive_for<T>(
        &mut self,
        addr: &Addr,
        reactive: T,
        timeout: Duration,
    ) -> Result<(), crate::Error>
    where
        T: Behavior + Send + 'static,
    {
        let boxed_reactive = Box::new(reactive);

        let addr = addr.clone();

        match self.execute_fn(
            move |disp| match disp.replace_reactive(&addr, boxed_reactive) {
                Ok(_) => Ok(()),
                Err(_) => Err(()),
            },
            timeout,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(())) => Err(crate::Error::ActorDisappeared),
            Err(e) => Err(e),
        }
    }

    /// Same as register_reactive_for(reactive, Duration::from_secs(60)) that panics if something goes wrong.
    /// To be used with a valid accessor.
    pub fn register_reactive_unwrap<T>(&mut self, reactive: T) -> Addr
    where
        T: Behavior + Send + 'static,
    {
        match self.register_reactive_for(
            reactive,
            // One minute reaction time is way beyond a not broken system.
            Duration::from_secs(60),
        ) {
            Ok(addr) => {
                if addr.is_valid() {
                    addr
                } else {
                    panic!("register_reactive_unwrap() failed because addr was INVALID");
                }
            }
            Err(e) => panic!("register_reactive_unwrap() failed with error {:?}", e),
        }
    }

    /// Same as replace_reactive_for(addr, reactive, Duration::from_secs(60)) that panics if something goes wrong.
    /// To be used with a valid accessor.
    pub fn replace_reactive_unwrap<T>(&mut self, addr: &Addr, reactive: T)
    where
        T: Behavior + Send + 'static,
    {
        match self.replace_reactive_for(
            addr,
            reactive,
            // One minute reaction time is way beyond a not broken system.
            Duration::from_secs(60),
        ) {
            Ok(()) => {}
            Err(e) => panic!("replace_reactive_unwrap() failed with error {:?}", e),
        }
    }

    /// Execute a function inside the thread of the dispatcher.
    /// Useful to add to register to the dispatcher actor that are not Send.
    pub fn execute_fn<F, T>(&mut self, f: F, timeout: Duration) -> Result<T, crate::Error>
    where
        F: FnOnce(&mut dyn Dispatcher) -> T + Send + 'static,
        T: Send + 'static + Sized,
    {
        let result = self.active_actor.request_for::<_, Box<dyn Any + Send>>(
            &self.disp_addr,
            Request::ExecuteFn {
                executable_fn: Cell::new(Box::new(move |disp| Box::new((f)(disp)))),
            },
            timeout,
        );

        match result {
            Ok(boxed) => match boxed.downcast::<T>() {
                Ok(t) => Ok(*t),
                Err(_) => Err(crate::Error::DowncastFailed),
            },
            Err(_) => todo!(),
        }
    }

    /// Join the thread of the dispatcher that must have been stopped by other means.
    /// Return the result of join().
    #[deprecated = "do join() manually if needed"]
    pub fn join_dispatcher_thread(&mut self) -> Result<(), Box<(dyn Any + Send + 'static)>> {
        Ok(())
    }

    /// Stop the dispatcher.
    /// Return the result from the request_for.
    pub fn stop_dispatcher(&mut self, timeout: Duration) -> Result<(), crate::Error> {
        if !self.disp_addr.is_valid() {
            return Ok(());
        }

        match self.active_actor.request_for::<_, Response>(
            &self.disp_addr,
            Request::StopDispatcher {},
            timeout,
        ) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }
}
