//! A framework to implement the reactive pattern on real-time systems.
//!
//! # Example of reactive actor executed by a dispatcher in its thread
//!
//! ```
//! use rtactor::dispatcher;
//! use rtactor::{spawn_dispatcher, ActiveMailbox, Addr, Behavior, Message, ProcessContext, send_notification};
//! use std::time::Duration;
//!
//!    // A very simple reactive actor that allows incrementing and querying an integer.
//!    struct TestReactive {
//!        pub val: i32,
//!    }
//!
//!    enum Notification {
//!        Increment(i32),
//!    }
//!
//!    enum Request {
//!        GetValue,
//!        ToString(String /*label*/),
//!    }
//!
//!    enum Response {
//!        GetValue(i32),
//!        ToString(String),
//!    }
//!
//!    impl Behavior for TestReactive {
//!        fn process_message<'a>(&mut self, context: &'a mut ProcessContext, msg: &Message) {
//!            match msg {
//!                Message::Notification(notif) => {
//!                    if let Some(notif) = notif.data.downcast_ref::<Notification>() {
//!                        match notif {
//!                            Notification::Increment(increment) => self.val += increment,
//!                        }
//!                    }
//!                }
//!                Message::Request(request) => {
//!                    if let Some(data) = request.data.downcast_ref::<Request>() {
//!                        match data {
//!                            Request::GetValue => {
//!                                context.send_response(request, Response::GetValue(self.val));
//!                            }
//!
//!                            Request::ToString(label) => context.send_response(
//!                                &request,
//!                                Response::ToString(format!("{label}: {}", self.val)),
//!                            ),
//!                        }
//!                    }
//!                }
//!                _ => panic!(),
//!            }
//!        }
//!    }
//!
//!    let initial_value = 0;
//!
//!    // Start a dispatcher inside its own thread.
//!    // The active object is created with the closure called inside the dispatcher thread.
//!    // This allows to have reactive object that are not movable between threads.
//!    let (dispatcher_addr, join_handle, test_reactive_addr) = spawn_dispatcher(10, move |disp| {
//!        // Create a reactive object on the heap.
//!        let test_reactive = Box::new(TestReactive { val: initial_value });
//!        // Move it inside the dispatcher and return the reactive address as the return of `setup_func`
//!        disp.register_reactive(test_reactive)
//!    });
//!
//!    send_notification(&test_reactive_addr, Notification::Increment(10))
//!    .unwrap();
//!
//!    // Create an active object to interact with the reactive under test.
//!    let mut prober = ActiveMailbox::new(1);
//!
//!    // Request the value.
//!    let result = prober.request_for::<_, Response>(
//!        &test_reactive_addr,
//!        Request::GetValue,
//!        Duration::from_secs(10),
//!    );
//!
//!    if let Ok(Response::GetValue(val)) = result {
//!        assert_eq!(val, 10);
//!    } else {
//!        panic!();
//!    }
//!
//!    // An other notification.
//!    send_notification(&test_reactive_addr, Notification::Increment(-3))
//!    .unwrap();
//!
//!    // An other different request.
//!    let result = prober.request_for::<_, Response>(
//!        &test_reactive_addr,
//!        Request::ToString("the value".to_string()),
//!        Duration::from_secs(10),
//!    );
//!    if let Ok(Response::ToString(s)) = result {
//!        assert_eq!(s, "the value: 7");
//!    } else {
//!        panic!();
//!    }
//!
//!    // Request to stop the dispatcher using its own address.
//!    let result = prober.request_for::<_, dispatcher::Response>(
//!        &dispatcher_addr,
//!        dispatcher::Request::StopDispatcher{},
//!        Duration::from_secs(10),
//!    );
//!    if let Ok(dispatcher::Response::StopDispatcher()) = result {
//!    } else {
//!        panic!();
//!    }
//!
//!    // Wait that the dispatcher thread finishes.
//!    join_handle.join().unwrap();
//! ```
//!
//! # Example of simulation of a reactive actor in a single threaded test
//!
//! ```
//! use rtactor::simulation::SimulationDispatcher;
//! use rtactor::{ActiveMailbox, Behavior, Message, ProcessContext, send_notification};
//! use std::time::Duration;
//!
//! // A very simple reactive actor that allows incrementing and querying an integer.
//! struct TestReactive {
//!     pub val: i32,
//! }
//!
//! enum Notification {
//!     Increment(i32),
//! }
//!
//! enum Request {
//!     GetValue,
//!     ToString(String /*label*/),
//! }
//!
//! enum Response {
//!     GetValue(i32),
//!     ToString(String),
//! }
//!
//! impl Behavior for TestReactive {
//!     fn process_message<'a>(&mut self, context: &'a mut ProcessContext, msg: &Message) {
//!         match msg {
//!             Message::Notification(notif) => {
//!                 if let Some(notif) = notif.data.downcast_ref::<Notification>() {
//!                     match notif {
//!                         Notification::Increment(increment) => self.val += increment,
//!                     }
//!                 }
//!             }
//!             Message::Request(request) => {
//!                 if let Some(data) = request.data.downcast_ref::<Request>() {
//!                     match data {
//!                         Request::GetValue => {
//!                             context.send_response(request, Response::GetValue(self.val))
//!                         }
//!
//!                         Request::ToString(label) => context.send_response(
//!                             &request,
//!                             Response::ToString(format!("{label}: {}", self.val)),
//!                         ),
//!                     }
//!                 }
//!             }
//!             _ => panic!(),
//!         }
//!     }
//! }
//!
//! // Create a simulation dispatcher.
//! let mut disp = SimulationDispatcher::new(10);
//!
//! // Create a reactive object on the heap.
//! let test_reactive = Box::new(TestReactive { val: 0 });
//!
//! // Move it inside the dispatcher. It starts the dispatch of messages for it.
//! let test_reactive_addr = disp.register_reactive(test_reactive);
//!
//! // Send a notification to the reactive.
//!     send_notification(&test_reactive_addr, Notification::Increment(10))
//!     .unwrap();
//!
//! // Create an active object to interact with the reactive under test.
//! let mut prober = ActiveMailbox::new(1);
//!
//! // Ask the simulation dispatcher to simulate a request by the active actor.
//! let result = disp.active_request_for::<_, Response>(
//!     &mut prober,
//!     &test_reactive_addr,
//!     Request::GetValue,
//!     Duration::from_secs(10),
//! );
//! if let Ok(Response::GetValue(val)) = result {
//!     assert_eq!(val, 10);
//! } else {
//!     panic!();
//! }
//!
//! // An other notification.
//!     send_notification(&test_reactive_addr, Notification::Increment(-3))
//!     .unwrap();
//!
//! // An other different request.
//! let result = disp.active_request_for::<_, Response>(
//!     &mut prober,
//!     &test_reactive_addr,
//!     Request::ToString("the value".to_string()),
//!     Duration::from_secs(10),
//! );
//! if let Ok(Response::ToString(s)) = result {
//!     assert_eq!(s, "the value: 7");
//! } else {
//!     panic!();
//! }
//!
//! // No need to stop the dispatcher, there is no thread, everything is single threaded.
//! // The reactive actor will be dropped by the drop of the simulation dispatcher.
//! ```
//!
//! # Doc about actors
//! A good explanation why the actor pattern is good for multitask:
//! <https://getakka.net/articles/intro/what-problems-does-actor-model-solve.html>
//!
//! A well designed rust actor framework (but not suitable for real-time):
//! <https://docs.rs/axiom/latest/axiom/>

pub extern crate rtactor_macros;

extern crate self as rtactor;

mod active;
mod actor;
mod reactive;

#[cfg(feature = "mockall")]
mod reactive_mocker;

pub mod dispatcher;
pub mod mpsc_dispatcher;
pub mod profiled_actor;
pub mod profiling_aggregator;
pub mod simulation;

pub use actor::send_notification;
pub use actor::Addr;
pub use actor::Error;
pub use actor::Message;
pub use actor::Notification;
pub use actor::Request;
pub use actor::RequestId;
pub use actor::Response;

#[deprecated = "use ActiveMailbox instead"]
#[allow(deprecated)]
pub use active::ActiveActor;

pub use active::ActiveMailbox;
pub use active::SyncAccessor;

pub use mpsc_dispatcher::spawn_dispatcher;
pub use mpsc_dispatcher::MpscDispatcher;
pub use reactive::Behavior;
pub use reactive::DummyBehavior;
pub use reactive::Instant;
pub use reactive::ProcessContext;
pub use reactive::Timeout;
pub use reactive::Timer;

#[cfg(feature = "async-actor")]
pub mod async_actor;

#[cfg(feature = "async-actor")]
pub use async_actor::{AsyncAccessor, AsyncMailbox};

#[cfg(feature = "mockall")]
pub use reactive::MockBehavior;

// This crate used in the interface is reexported to allow user to build with the same version.
// see https://doc.rust-lang.org/rustdoc/write-documentation/re-exports.html
#[cfg(feature = "mockall")]
pub use mockall;
