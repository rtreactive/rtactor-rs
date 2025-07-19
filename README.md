<!-- markdownlint-disable MD033 -->
# RTactor framework

An *Actor* framework specially designed for *R*eal-*T*ime constrained use
cases.

The framework provides:

* *Reactive* actors that delegate the processing to a designed *dispatcher*
* *Active* actors that freely manage their input queues
* A `SimulationDispatcher` that replaces the normal dispatcher to do single
   threaded unit tests and supports timing simulations
* Proc-macros that turn the enums of the message definitions into *SyncAccessor*
   synchronous interfaces that can be used at run time or in simulations

Table of contents:

1. [Why another actor framework?](#h2_why)
2. [Basic concepts of `rtactor`](#h2_basic_concepts)
3. [A minimal example of a reactive actor](#h2_minimal_example)
4. [A more realistic example with unit tests](#h2_realistic_example)
5. [Useful patterns](#h2_useful_patterns)
6. [Limitations and possible improvements](#h2_limitations)

## Why another actor framework? <a name="h2_why"></a>

### Existing frameworks are mostly IO-bound

Async runtimes (such as Tokio) and actor frameworks built on top of them are
primarily designed for IO-bound, best effort, maximum throughput applications
such as servers with a lot of concurrent requests.

The idea is to have a pool of threads in the range of the number of cores and
schedule asynchronous processing of the actor behaviors in such a way that no
cores ever idle.
When there are a lot more messages to be processed than there are worker
threads, this greatly reduces the number of context switches compared to a
threaded application.
In this model, the developer delegates the management of threads (priority,
number, migration) to the framework.
The asynchronous code should also be written in a way that allows migration to
another thread.

This model is very efficient to achieve maximum throughput (operations per
second averaged over a long period with a typical workload).

### We needed real-time and testability

For applications with a real-time requirement, such as live audio, video, or
digital control, the goal is to guarantee a maximum processing duration in the
worst case scenario.
To achieve that, time critical processing is explicitly dispatched to threads
with higher priorities. Load balancing and migration are neither needed nor
wanted.

This makes the use of async runtimes impractical for truly real-time code.

However, the pattern of actors reacting to asynchronous messages is still very
attractive for real-time systems. Very often an entity will receive data at
different rates from different sources and send stimuli to many destinations.
The traditional blocking model of programming is very efficient for parser or
compiler style problems, processing of data based on an algorithm.
Signal processing or embedded systems problems generally consist of many
entities that exchange data at different rates. Thread-safe,
blocking code for these kinds of systems generally ends up implemented as queues
protected by mutexes wrapped in blocking interfaces.
Waiting on many sources is then difficult and the multi-threaded nature of the
code makes it difficult or impossible to unit test.

`rtactor` is a solution for building actor-based real-time systems with easy
unit testing.

## Basic concepts of `rtactor` <a name="h2_basic_concepts"></a>

An actor is only reachable through its address. The `Addr` struct can be
freely copied and exchanged between threads. It can be invalid.

Three types of messages can be sent to actors: `Request` and its mandatory
`Response`, or `Notification` that does not expect any response.
The request-response transaction has a `RequestId` that allows rejecting
duplicates.

The three message types have data attached to them. They can have any Rust type and
they are specific to a given actor. It is recommended to create an interface file for
a given abstract actor type. The data consists of `enum`s that allow to
differentiate between request, response or notification types.

A reactive actor should implement the `Behavior` trait that allows it to react
to received messages in the `process_message()` method. Depending on the
real-time constraints, several dispatchers are created and each reactive actor
is moved into one of them. From this point, only the address is used to send
requests or notifications to the actor.

An active actor is simply a queue of messages that the user code should
constantly empty.

A special type of dispatcher is used for unit tests, which allows the
dispatcher to process messages for a given duration. Because everything is
executed in the thread of the unit test, it removes thread-safety concerns from
unit tests. The time is simulated, so that the unit test does not depend on the
execution speed and use cases with very long durations can be tested.

## A minimal example of a reactive actor <a name="h2_minimal_example"></a>

Let's start with a very simple example that does almost nothing. A `Client` struct and
`Server` struct are both reactive actors managed by the same dispatcher.
When the `Client` receives a `Notification` message, it performs a request
on the `Server`, displays the `Response` data it receives, and then stops
the dispatcher.

For the sake of simplicity, the `Client` is going to receive a simple `char` as
data of the `Notification`, send a `String` in the `Request`, and receive an i32
in the `Response`. In a real example it would make sense to use enums encapsulating
the data for more clarity and to allow different Message types.

The code below can be found in [`tests/readme_minimal_example.rs`](tests/readme_minimal_example.rs).

```rust
use rtactor as rt;

struct Client {
    server_address: rt::Addr,
    dispatcher_address: rt::Addr,
    request_id: rt::RequestId,
}

impl Client {
    fn new(server_address: rt::Addr, dispatcher_address: rt::Addr) -> Client {
        Client {
            server_address,
            dispatcher_address,
            request_id: rt::RequestId::default(),
        }
    }
}

impl rt::Behavior for Client {
    fn process_message(&mut self, context: &mut rt::ProcessContext, message: &rt::Message) {
        match message {
            rt::Message::Notification(notification) => {
                if let Some(c) = notification.data.downcast_ref::<char>() {
                    println!("The client received a Notification with data='{}'.", c);
                    let str = format!("c={}", c);
                    self.request_id = context.send_request(&self.server_address, str);
                }
            }
            rt::Message::Response(response) => {
                if response.id_eq(self.request_id) {
                    if let Some(val) = response.result.as_ref().unwrap().downcast_ref::<i32>() {
                        println!("The client received a Response with result={}.", val);

                        context.send_request(
                            &self.dispatcher_address,
                            rt::dispatcher::Request::StopDispatcher {},
                        );
                        println!("The dispatcher will now stop.");
                        // The dispatcher will be stopped after that point
                        // so the response will never be processed.
                    }
                }
            }
            _ => panic!(),
        }
    }
}

struct Server();

impl rt::Behavior for Server {
    fn process_message(&mut self, context: &mut rt::ProcessContext, message: &rt::Message) {
        match message {
            rt::Message::Request(request) => {
                if let Some(str) = request.data.downcast_ref::<String>() {
                    println!("The server received a Request with data=\"{}\".", str);
                    context.send_response(request, 42i32);
                }
            }
            _ => panic!(),
        }
    }
}

#[test]
fn test() {
    use std::time::Duration;

    let disp_builder = rt::mpsc_dispatcher::Builder::new(10);
    let mut disp_accessor = disp_builder.to_accessor();

    let join_handle = std::thread::spawn(move || disp_builder.build().process());

    // The way an actor that is Send can be registered.
    let server_addr = disp_accessor
        .register_reactive_for(Server(), Duration::from_secs(10))
        .unwrap();

    // Registering inside the dispatcher thread is a bit more complex but allow to use actor
    // that are not Send.
    let client_addr = disp_accessor
        .execute_fn(
            move |disp| disp.register_reactive(Box::new(Client::new(server_addr, disp.addr()))),
            Duration::from_secs(10),
        )
        .unwrap();

    rt::send_notification(&client_addr, '_').unwrap();

    join_handle.join().unwrap();
}
```

## A more realistic example with unit tests <a name="h2_realistic_example"></a>

Let's take the simple use case of an actor that receives a data flow from an accelerometer
and regularly sends the average acceleration on a CAN bus.

The excerpts below were taken from [`tests/readme_realistic_example.rs`](tests/readme_realistic_example.rs).

### Interface definitions

Let's first define the interface of the system. For that, a module called
`acceleration_broadcaster` is created. It will define the enum and structs
used to communicate with any implementation of the actor. An enum called
`Request` is created to describe all request kinds with `{}` named enum values
for the arguments of the requests. There needs to be a corresponding `Response` enum with
the same variant names. Note that the macro `ResponseEnum` is used here to
automatically create the enum. The `Start` and `Stop` requests are defined.

For data that does not need a response, a notification enum is created. In
this case this is `AccelerationSample` to receive data from the accelerometer.

A so-called "Sync" notifier, requester, "SimSyncAccessor" and "SyncAccessor" are also defined with
the help of macros. They will help us to access the broadcaster actor in a
synchronous way in the unit tests (more details about this later).

```rust
pub mod acceleration_broadcaster {
    use rtactor as rt;

    use rt::{
        define_sim_sync_accessor, define_sync_accessor,
        rtactor_macros::{ResponseEnum, SyncNotifier, SyncRequester},
    };

    #[derive(ResponseEnum, SyncRequester)]
    pub enum Request {
        Start { can_controller_addr: rt::Addr },
        Stop {},
    }
    #[derive(SyncNotifier)]
    pub enum Notification {
        AccelerationSample { acceleration: [i16; 3] },
    }

    define_sim_sync_accessor!(SimSyncAccessor, SyncRequester, SyncNotifier);
    define_sync_accessor!(SyncAccessor, SyncRequester, SyncNotifier);
}
```

Similarly, the interface of the actor responsible for the CAN bus transmission is defined.
To add a return value to a request response, an annotation in the form of `#[response_val(type)]`
is added.

```rust
pub mod can_controller {
    use rtactor::rtactor_macros::ResponseEnum;

    #[derive(Debug)]
    pub enum Error {
        SendFailed,
        InvalidId,
        InvalidLength,
    }

    #[derive(ResponseEnum)]
    pub enum Request {
        #[response_val(Result<(), Error>)]
        SendMessage { id: u32, length: u8, data: [i16; 4] },
    }
}
```

### Top-level implementation of the actor

We are going to implement the actor as a reactive entity. This means that
the actor reacts to messages instead of actively waiting on them. In practice,
the actor is placed under the responsibility of a dispatcher. When a message
is sent to the actor, it is placed in the queue of the dispatcher. The
dispatcher then calls, in its own thread, the `process_message()`
method of the actor.

The reactive actor is a simple struct that implements `rtactor::Behavior`. In our
case, the actor keeps track of the CAN controller actor address, counters for the
averaging, and the state of the actor. We also have a reference to a timer that
allows the dispatcher to send a notification periodically.

```rust
pub mod simple_acceleration_broadcaster {
    use rtactor as rt;

    use std::time::Duration;

    use crate::{acceleration_broadcaster, can_controller};

    enum State {
        Stopped,
        WaitSamples,
        SendingCanMessage { request_id: rt::RequestId },
    }

    pub struct Broadcaster {
        can_controller_addr: rt::Addr,
        sample_count: u32,
        acceleration_sum: [i32; 3],
        state: State,
        timer: rt::Timer,
    }
```

The top level of processing of messages is the method
`process_message()` of the trait `rtactor::Behavior`.
For convenience and readability, the messages are
distinguished by their variants (`Request`, `Response`, or `Notification`)
and then by their data. Then, because the message data is a `dyn Any`, it is possible
to use `downcast_ref()` and pattern matching to separate the messages
and let specific methods handle them. The timer is a special case,
because it provides a method `is_scheduling()` that allows safely checking
whether the timer `Notification` was expected.

```rust
impl rt::Behavior for Broadcaster {
    fn process_message(&mut self, context: &mut rt::ProcessContext, msg: &rt::Message) {
        match msg {
            rt::Message::Request(request) => {
                if let Some(req_data) = request.data.downcast_ref() {
                    self.process_broadcaster_request(context, request, req_data);
                }
            }
            rt::Message::Response(response) => {
                if let Ok(resp_data) = response.result.as_ref() {
                    if let Some(resp_data) = resp_data.downcast_ref() {
                        self.process_can_controller_response(response, resp_data);
                    }
                }
            }
            rt::Message::Notification(notification) => {
                if let Some(notif_data) = notification.data.downcast_ref() {
                    self.process_accelerometer_notification(notif_data);
                } else if self.timer.is_scheduling(notification) {
                    self.process_timer_elapsed(context);
                }
            }
        }
    }
}
```

### Implementation of the reacting to different messages

In `impl Broadcaster`, the method `process_broadcaster_request()` reacts
to requests of the interface of the `accelerometer_broadcaster`. It is
important to always respond to a request, so that the control flow of the requester is not
broken. For a `Start` variant request, the timer is started with the broadcast period.
In case of a `Stop`, the timer is stopped with `unschedule`. For a reactive actor,
many operations are performed with the `ProcessContext` passed to
`process_message()`. This helps to keep the `Timer` struct lightweight.

```rust
fn process_broadcaster_request(
    &mut self,
    context: &mut rt::ProcessContext,
    request: &rt::Request,
    data: &acceleration_broadcaster::Request,
) {
    match data {
        acceleration_broadcaster::Request::Start {
            can_controller_addr,
        } => {
            if let State::Stopped = self.state {
                self.can_controller_addr = can_controller_addr.clone();
                context.schedule_for(&mut self.timer, BROADCAST_PERIOD);
                self.state = State::WaitSamples;
                self.reset_sum();
            }
            context.send_response(request, acceleration_broadcaster::Response::Start());
        }
        acceleration_broadcaster::Request::Stop {} => {
            if let State::Stopped = self.state {
            } else {
                context.unschedule(&mut self.timer);
                self.state = State::Stopped;
                self.reset_sum();
            }
            context.send_response(request, acceleration_broadcaster::Response::Stop());
        }
    }
}
```

The reception of the acceleration is very simple and consists of:

```rust
fn process_accelerometer_notification(
    &mut self,
    data: &acceleration_broadcaster::Notification,
) {
    let acceleration_broadcaster::Notification::AccelerationSample { acceleration } = data;
    for (i, a) in acceleration.iter().enumerate() {
        self.acceleration_sum[i] += *a as i32;
    }
    self.sample_count += 1;
}
```

When the timer is ready, the average acceleration is crudely computed and a
request is sent to the CAN controller actor. We keep the identifier of
the request and change to a state where the response is expected. The
`request_id` allows to reject old responses unrelated to the current request.
In this implementation, if the CAN bus is too slow (state is still `SendingCanMessage`
when the next timer elapses), the CAN message is simply not sent.

```rust
fn process_timer_elapsed(&mut self, context: &mut rt::ProcessContext) {
    if let State::WaitSamples = self.state {
        if self.sample_count > 0 {
            let mut data = [0i16; 4];
            for (i, a_sum) in self.acceleration_sum.into_iter().enumerate() {
                let a = a_sum / (self.sample_count as i32);

                data[i] = a as i16;
            }

            let request_id = context.send_request(
                &self.can_controller_addr,
                can_controller::Request::SendMessage {
                    id: CAN_ACCELERATION_ID,
                    length: 3,
                    data,
                },
            );

            self.state = State::SendingCanMessage { request_id };
        }
    }

    self.reset_sum();
    context.schedule_for(&mut self.timer, BROADCAST_PERIOD);
}
```

When the controller response is received, the state goes
back to `WaitSamples`. As an example, if there is an
application error, the error is printed to `stdout`.

```rust
fn process_can_controller_response(
    &mut self,
    response: &rt::Response,
    data: &can_controller::Response,
) {
    let can_controller::Response::SendMessage(result) = data;

    if let State::SendingCanMessage { request_id } = self.state {
        if response.id_eq(request_id) {
            if let Err(error) = result {
                println!("can error: {error:?}");
            }
            self.state = State::WaitSamples;
        }
    }
}
```

### Unit testing the reactive actor

To build a test of one or more reactive actors, the struct `SimulationDispatcher`
is provided. Instead of the normal `MpscDispatcher` that performs
message processing in its own thread, the dispatch is requested by
the test code in the thread of the test. This avoids the non-reproducibility
of multithreaded test code, and allows to use single-threaded test libraries and simplifies
test debugging.

Instead of using the system time for timeouts and timers,
a simulated time base is used. The processing time is simulated as
instantaneous, something that greatly simplifies writing tests. In this way,
timing assertions are deterministic and do not depend on the execution time
of the unit tests. This allows to test very short timing accurately
and very long timing use cases in tests that execute almost instantaneously.

Here is an example of test from [`tests/readme_realistic_example.rs`](
    tests/readme_realistic_example.rs):

```rust
#[test]
fn test_broadcaster_simulated() {
    const ACCELEROMETER_T_SAMPLE: Duration = Duration::from_millis(50);

    // Create the simulation dispatcher. To use "SimSyncAccessor" it
    // has to be wrapper in a RefCell.
    let disp = RefCell::new(rt::simulation::SimulationDispatcher::new(10));

    // Create the broadcaster actor in a Box. This allows to move it
    // in the dispatcher with `register_reactive`. The actor is now
    // managed by the dispatcher. The address of the broadcaster is returned.
    let broadcaster_addr = disp
        .borrow_mut()
        .register_reactive(Box::new(simple_acceleration_broadcaster::Broadcaster::new()));

    // Create a struct that allows to access synchronously to the broadcaster
    // request and notification. This makes writing tests much less verbose.
    let mut broadcaster_accessor =
        acceleration_broadcaster::SimSyncAccessor::new(&disp, &broadcaster_addr);

    // Create an active actor to simulate the CAN controller. Here the
    // actor interface is used to fake the behavior. It is also possible to
    // use mock created with mock libs like `mockall`.
    let mut fake_can = rt::ActiveMailbox::new(10);

    // It is necessary to bring these trait to the scope to use them.
    use crate::acceleration_broadcaster::{SyncNotifier, SyncRequester};

    let begin = disp.borrow_mut().now();

    // Use the accessor to start the broadcaster. It is expected that
    // the execution is without delay. This is possible because in the simulation
    // the CPU processing take no time.
    broadcaster_accessor
        .start(fake_can.addr(), Duration::ZERO)
        .unwrap();

    // Make the simulation advance half the accelerometer sampling rate
    // so the can message is guaranteed to be received in the test bellow.
    // Any queued message is processed in `process_for` (but in this case none should be).
    disp.borrow_mut().process_for(ACCELEROMETER_T_SAMPLE / 2);

    for i in 0..3 {
        for j in 0..10 {
            println!(
                "i={i}, j={j} {:?}",
                disp.borrow_mut().now().saturating_sub(&begin)
            );

            // Use the accessor to send the accelerometer notification.
            broadcaster_accessor
                .acceleration_sample([-1000, 2000, 3000])
                .unwrap();
            disp.borrow_mut().process_for(ACCELEROMETER_T_SAMPLE);
        }

        // It is very important to use the method `active_wait_message*` of the dispatcher
        // when working with active actors. This insures processing of the queued messages
        // in the dispatcher and use of the simulated time. It's not the case if
        // `ActiveMailbox::wait_message*` methods of `ActiveMailbox` are used.
        let msg = disp
            .borrow_mut()
            .active_wait_message_for(&mut fake_can, Duration::ZERO)
            .unwrap();

        if let rt::Message::Request(request) = msg {
            if let Some(can_controller::Request::SendMessage { id, length, data }) =
                request.data.downcast_ref()
            {
                assert_eq!(*id, 0x1000);
                assert_eq!(*length, 3);
                assert_eq!(data[0], -1000);
                assert_eq!(data[1], 2000);
                assert_eq!(data[2], 3000);
            } else {
                panic!();
            }

            // Simulate the response.
            fake_can
                .responds(request, can_controller::Response::SendMessage(Ok(())))
                .unwrap();
        } else {
            panic!();
        }
    }

    broadcaster_accessor.stop(Duration::ZERO).unwrap();
}
```

### Running the reactive actor in the real world

Now that the correct behavior of the reactive actor has been tested
it can be used in application code. The choice of which reactive
goes to which dispatcher depends on the real-time constrains and is
up to the developer. `SyncAccessor` generated by `define_sync_accessor!()`
can be used to start and stop the system synchronously.

Here is an example from [`tests/readme_realistic_example.rs`](tests/readme_realistic_example.rs):

```rust
#[test]
fn test_broadcaster_threaded() {
    // Max real time duration for a dispatcher operation. For example
    // CI tasks can be frozen by other activities and lead to
    // test falsely failing.
    const MAX_EXEC_DURATION: Duration = Duration::from_secs(5);

    // Start a dispatcher in it own thread. The creation of one or
    // many reactives is done with a FnOnce called inside the thread
    // of the dispatcher. This allows to keep the construction single
    // threaded, simplifying the use of single threaded libs and code.
    // `spawn_dispatcher` returns as third value the return of the FnOnce,
    // here the broadcaster address.

    let builder = rt::mpsc_dispatcher::Builder::new(10);
    let mut disp_accessor = builder.to_accessor();

    let join_handle = std::thread::spawn(|| builder.build().process());

    let mut broadcaster_accessor = acceleration_broadcaster::SyncAccessor::new(
        &disp_accessor
            .register_reactive_unwrap(simple_acceleration_broadcaster::Broadcaster::new()),
    );
    use crate::acceleration_broadcaster::SyncRequester;

    let fake_can = rt::ActiveMailbox::new(10);

    // Simply start and stop the broadcaster.
    broadcaster_accessor
        .start(fake_can.addr(), MAX_EXEC_DURATION)
        .unwrap();
    broadcaster_accessor.stop(MAX_EXEC_DURATION).unwrap();

    // Ask the dispatcher to stop its operations.
    disp_accessor.stop_dispatcher(MAX_EXEC_DURATION).unwrap();

    // It is not strictly necessary to join() here because stop_dispatcher()
    // insure that all behaviors are destructed and all non processed Request
    // responded with an error. The thread after it will only live a short time
    // to destroy the dispatcher and its helpers (TimeoutManager, ProcessContext).
    // So it make sense only to wait that the memory used is given back to the system,
    // seldom a concern.
    join_handle.join().unwrap();
}
```

## Useful patterns <a name="h2_useful_patterns"></a>

### Build a system of actors with circular references

```rust
let sink_addr = disp_accessor.register_reactive_unwrap(rt::DummyBehavior::default());
let source_addr = disp_accessor.register_reactive_unwrap(Source::new(&sink_addr));
disp_accessor.replace_reactive_unwrap(&sink_addr, Sink::new(&source_addr));
```

## Limitations and possible improvements <a name="h2_limitations"></a>

Currently, the most problematic limitation is that messages use dynamic
allocation (`Box` that uses the heap under the hood). The heap allocation is not
real-time but is considered very fast. The use of memory pools could be a
solution in a future version.

Another problem is that a peer flooding an actor with messages can influence
the internal real-time behavior of this actor. A possible solution would be
using two queues with different priorities.

Finally, the queue size of the dispatcher needs to be specified at construction
time.
With systems without memory constraints, a large enough size could be used. It
would be better in the future to use a linked list of messages instead of a
fixed size queue with data from the heap.
The use of messages embedding the data and the pointers of the linked list,
allocated from a memory pool, is a possible solution to have to only handle
(and test) the problem of pool exhaustion.

### Rust version

This project is tested with rust `1.75.0` and rust `1.86.0`.
Version in between were not tested but should work.
