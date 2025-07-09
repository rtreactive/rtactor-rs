use assert2::{assert, check, let_assert};
use panic_context::*;
use rtactor::simulation::SimulationDispatcher;
use rtactor::{
    dispatcher, send_notification, ActiveActor, Addr, Behavior, Message, ProcessContext, Timer,
};
use std::vec::Vec;

use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

#[test]
fn empty_simulation_dispatcher() {
    let mut disp = SimulationDispatcher::new(10);
    disp.process_for(Duration::from_secs(1));
    disp.process_until(disp.now() + Duration::from_micros(1));
}

#[test]
/// A simple example code of a reactive actor simulation.
///
/// A example of this code  with a non-simulated dispatcher exists
/// in `tests/reactive.rs:simple_threaded_dispatcher()`.
fn simple_simulation_dispatcher() {
    // A very simple reactive actor that allows incrementing and querying an integer.
    struct TestReactive {
        pub val: i32,
    }

    enum Notification {
        Increment(i32),
    }

    enum Request {
        GetValue,
        ToString(String /*label*/),
    }

    enum Response {
        GetValue(i32),
        ToString(String),
    }

    impl Behavior for TestReactive {
        fn process_message(&mut self, context: &mut ProcessContext, msg: &Message) {
            match msg {
                Message::Notification(notif) => {
                    if let Some(notif) = notif.data.downcast_ref::<Notification>() {
                        match notif {
                            Notification::Increment(increment) => self.val += increment,
                        }
                    }
                }
                Message::Request(request) => {
                    if let Some(data) = request.data.downcast_ref::<Request>() {
                        match data {
                            Request::GetValue => {
                                context.send_response(request, Response::GetValue(self.val))
                            }

                            Request::ToString(label) => context.send_response(
                                request,
                                Response::ToString(format!("{label}: {}", self.val)),
                            ),
                        }
                    }
                }
                _ => panic!(),
            }
        }
    }

    // Create a simulation dispatcher.
    let mut disp = SimulationDispatcher::new(10);

    // Create a reactive object on the heap.
    let test_reactive = Box::new(TestReactive { val: 0 });

    // Move it inside the dispatcher. It starts the dispatch of messages for it.
    let test_reactive_addr = disp.register_reactive(test_reactive);

    // Send a notification to the reactive.

    send_notification(&test_reactive_addr, Notification::Increment(10)).unwrap();

    // Create an active object to interact with the reactive under test.
    let mut prober = ActiveActor::new(1);

    // Ask the simulation dispatcher to simulate a request by the active actor.
    let result = disp.active_request_for::<_, Response>(
        &mut prober,
        &test_reactive_addr,
        Request::GetValue,
        Duration::from_secs(10),
    );
    if let Ok(Response::GetValue(val)) = result {
        assert_eq!(val, 10);
    } else {
        panic!();
    }

    // An other notification.
    send_notification(&test_reactive_addr, Notification::Increment(-3)).unwrap();

    // An other different request.
    let_assert!(
        Ok(Response::ToString(str)) = disp.active_request_for(
            &mut prober,
            &test_reactive_addr,
            Request::ToString("the value".to_string()),
            Duration::from_secs(1)
        )
    );
    assert!(str == "the value: 7");

    // No need to stop the dispatcher, there is no thread, everything is single threaded.
    // The reactive actor will be dropped by the drop of the simulation dispatcher.
}

/// Schedule some timeout and check that it works properly.
#[test]
fn timer_test() {
    struct Tester {
        begin: rtactor::Instant,
        timers: Vec<Timer>,
        index: usize,
        observer: Addr,
    }
    enum Notification {
        Start(),
        Finished(),
    }

    impl Tester {
        pub fn new(observer_addr: &rtactor::Addr) -> Tester {
            Tester {
                timers: Vec::new(),
                begin: rtactor::Instant::INFINITY,
                index: 0,
                observer: observer_addr.clone(),
            }
        }
    }
    impl Behavior for Tester {
        fn process_message(&mut self, context: &mut ProcessContext, msg: &Message) {
            match msg {
                Message::Request(_) => panic!(),
                Message::Response(_) => panic!(),
                Message::Notification(notif) => {
                    if let Some(notif) = notif.data.downcast_ref() {
                        match notif {
                            Notification::Start() => {
                                // Schedule a number of timeouts.

                                let now = context.now();
                                self.begin = now;
                                for _ in 0..5 {
                                    self.timers.push(Timer::new())
                                }
                                context.schedule_until(
                                    &mut self.timers[0],
                                    now + Duration::from_millis(5),
                                );
                                context.schedule_until(
                                    &mut self.timers[1],
                                    now + Duration::from_millis(6),
                                );
                                context.schedule_until(
                                    &mut self.timers[2],
                                    now + Duration::from_millis(0),
                                );
                                context.schedule_until(
                                    &mut self.timers[3],
                                    now + Duration::from_millis(55),
                                );
                                context.schedule_until(
                                    &mut self.timers[4],
                                    now + Duration::from_millis(230),
                                );
                            }
                            Notification::Finished() => panic!(),
                        }
                    } else if self.timers[2].is_scheduling(notif) {
                        assert_eq!(self.index, 0);
                        assert!(
                            context.now().saturating_sub(&self.begin) >= Duration::from_millis(0)
                        );
                        self.index += 1;

                        for i in 0..self.timers.len() {
                            panic_context!("i={i}");
                            if i != 2 {
                                assert!(!self.timers[i].is_scheduling(notif));
                            } else {
                                assert!(self.timers[i].is_scheduling(notif))
                            }
                        }
                    } else if self.timers[0].is_scheduling(notif) {
                        check!(self.index == 1);
                        assert!(
                            context.now().saturating_sub(&self.begin) == Duration::from_millis(5)
                        );
                        self.index += 1;
                    } else if self.timers[1].is_scheduling(notif) {
                        check!(self.index == 2);
                        check!(
                            context.now().saturating_sub(&self.begin) == Duration::from_millis(6)
                        );
                        self.index += 1;
                    } else if self.timers[3].is_scheduling(notif) {
                        check!(self.index == 3);
                        check!(
                            context.now().saturating_sub(&self.begin) == Duration::from_millis(55)
                        );
                        self.index += 1;
                    } else if self.timers[4].is_scheduling(notif) {
                        assert!(
                            context.now().saturating_sub(&self.begin) == Duration::from_millis(230)
                        );
                        context
                            .send_notification(&self.observer, Notification::Finished())
                            .unwrap();
                    }
                }
            }
        }
    }

    let mut observer = ActiveActor::new(1);

    let mut disp = SimulationDispatcher::new(1);

    let tester_addr = disp.register_reactive(Box::new(Tester::new(&observer.addr())));

    observer
        .send_notification(&tester_addr, Notification::Start())
        .unwrap();

    let_assert!(
        Result::Ok(Message::Notification(notif)) =
            disp.active_wait_message_for(&mut observer, TEST_TIMEOUT)
    );
    let_assert!(Some(Notification::Finished()) = notif.data.downcast_ref());

    let_assert!(
        Result::Ok(dispatcher::Response::StopDispatcher()) = disp.active_request_for(
            &mut observer,
            &disp.addr(),
            dispatcher::Request::StopDispatcher {},
            TEST_TIMEOUT,
        )
    );
}

#[test]
fn test_replace_reactive() {
    // Create a simulation dispatcher.
    let mut disp = SimulationDispatcher::new(10);

    // Create a reactive object on the heap.
    let test_reactive_1 = Box::<rtactor::DummyBehavior>::default();

    // Move it inside the dispatcher. It starts the dispatch of messages for it.
    let test_reactive_addr = disp.register_reactive(test_reactive_1);

    // Create another reactive object.
    let test_reactive_2 = Box::<rtactor::DummyBehavior>::default();

    // Replace the first reactive by the second.
    let result = disp.replace_reactive(&test_reactive_addr, test_reactive_2);
    match result {
        Ok(_behaviour) => {
            // TODO: check that the returned behaviour is the first one.
        }
        Err(_) => panic!("Error replacing the reactive."),
    }
}
