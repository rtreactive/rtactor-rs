use assert2::{assert, let_assert};
use oorandom::{self};
use rstest::rstest;
use rtactor as rt;
use rtactor::dispatcher;
use rtactor::{
    self, send_notification, spawn_dispatcher, ActiveMailbox, Addr, Behavior, Message,
    ProcessContext, RequestId, Timer,
};
use std::cmp::max;
use std::collections::BTreeMap;
use std::thread::JoinHandle;
use std::time::Duration;

struct CountdownReactive {
    pub disp_addr: Addr,
    pub counter: u32,
}

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

enum Notification {
    Start(u32), // counter value
    Decrement,
}

impl CountdownReactive {
    pub fn new(disp_addr: &Addr) -> CountdownReactive {
        CountdownReactive {
            disp_addr: disp_addr.clone(),
            counter: 0,
        }
    }
}

impl Behavior for CountdownReactive {
    fn process_message(&mut self, context: &mut ProcessContext, msg: &Message) {
        match msg {
            Message::Notification(notif) => {
                if let Some(notif) = notif.data.downcast_ref::<Notification>() {
                    match notif {
                        Notification::Start(counter) => {
                            self.counter = *counter;
                            context
                                .send_self_notification(Notification::Decrement)
                                .unwrap();
                        }
                        Notification::Decrement => {
                            self.counter -= 1;
                            if self.counter == 0 {
                                let _request_id = context.send_request(
                                    &self.disp_addr,
                                    dispatcher::Request::StopDispatcher {},
                                );
                                // do not handle response has we are not interested when stopped
                            } else {
                                // continue to decrement
                                context
                                    .send_self_notification(Notification::Decrement)
                                    .unwrap();
                            }
                        }
                    }
                }
            }
            _ => panic!(),
        }
    }
}

#[test]
fn start_stop_empty_dispatcher() {
    let (addr, join_handle, _) = spawn_dispatcher(10, |_| {});

    let mut stopper = ActiveMailbox::new(1);
    let response = stopper.request_for::<_, dispatcher::Response>(
        &addr,
        dispatcher::Request::StopDispatcher {},
        Duration::from_secs(60),
    );

    match response {
        Ok(dispatcher::Response::StopDispatcher()) => (),
        Ok(_) => panic!(),
        Err(_) => panic!(),
    }

    assert!(join_handle.join().is_ok());
}

#[test]
fn start_stop_dispatcher_accessor() {
    let builder = rt::mpsc_dispatcher::Builder::new(10);
    let mut disp_accessor = builder.to_accessor();
    std::thread::spawn(move || builder.build().process());

    disp_accessor
        .stop_dispatcher(Duration::from_secs(100))
        .unwrap();
}

#[rstest]
/// Test the dispatcher with a reactive that count down and stop the dispatcher.
fn reactive_countdown(#[values(false, true)] replace_behavior: bool) {
    let builder = rt::mpsc_dispatcher::Builder::new(10);
    let mut disp_accessor = builder.to_accessor();
    let join_handle = std::thread::spawn(move || builder.build().process());

    let reactive = CountdownReactive::new(disp_accessor.dispatcher_addr());
    let reactive_addr = if replace_behavior {
        let addr = disp_accessor.register_reactive_unwrap(rt::DummyBehavior::default());
        disp_accessor.replace_reactive_unwrap(&addr, reactive);
        addr
    } else {
        disp_accessor.register_reactive_unwrap(reactive)
    };

    rtactor::send_notification(&reactive_addr, Notification::Start(3)).unwrap();

    // Wait that the thread is terminated by the actor.
    join_handle.join().unwrap();

    // The actor and the dispatcher should stop to be reachable.
    match send_notification(&reactive_addr, Notification::Start(3)).unwrap_err() {
        rt::Error::AddrUnreachable => {}
        e => panic!("expect AddrUnreachable, not a {e:?}"),
    };
    match disp_accessor
        .stop_dispatcher(Duration::from_secs(100))
        .unwrap_err()
    {
        rt::Error::AddrUnreachable => {}
        e => panic!("expect AddrUnreachable, not a {e:?}"),
    };
}

#[test]
/// A simple example code of a reactive actor with it own dispatcher.
///
/// A simulation of this example exists in `tests/simulation.rs:simple_simulation_dispatcher()`.
fn simple_threaded_dispatcher() {
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
                                context.send_response(request, Response::GetValue(self.val));
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

    let initial_value = 0;

    let builder = rt::mpsc_dispatcher::Builder::new(10);
    let mut dispatcher_accessor = builder.to_accessor();
    std::thread::spawn(move || builder.build().process());

    let test_reactive_addr =
        dispatcher_accessor.register_reactive_unwrap(TestReactive { val: initial_value });

    send_notification(&test_reactive_addr, Notification::Increment(10)).unwrap();

    // Create an active object to interact with the reactive under test.
    let mut prober = ActiveMailbox::new(1);

    // Request the value.
    let_assert!(
        Ok(Response::GetValue(val)) = prober.request_for(
            &test_reactive_addr,
            Request::GetValue,
            Duration::from_secs(10),
        )
    );
    assert!(val == 10);

    // An other notification.
    let_assert!(Ok(()) = send_notification(&test_reactive_addr, Notification::Increment(-3)));

    // An other different request.
    let_assert!(
        Ok(Response::ToString(str)) = prober.request_for(
            &test_reactive_addr,
            Request::ToString("the value".to_string()),
            Duration::from_secs(10),
        )
    );
    assert!(str == "the value: 7");

    // Request to stop the dispatcher using its own address.
    dispatcher_accessor
        .stop_dispatcher(Duration::from_secs(60))
        .unwrap();

    // Try to reach after stopping the dispatcher.
    let_assert!(
        Err(err) = prober.request_for::<_, Response>(
            &test_reactive_addr,
            Request::GetValue,
            Duration::from_secs(10)
        )
    );
    match err {
        rt::Error::AddrUnreachable | rt::Error::ActorDisappeared => (),
        _ => panic!(),
    }
}

#[test]
/// A multi-thread stress test of message passing.
///
/// A certain number of reactives send a number of message randomly affected to them
/// and are randomly affected to a number of dispatchers.
/// They send all the messages configured in `emitting_exchange_confs` and at the same
/// time receive all the message expected in `receiving_exchange_confs`. There is a counter
/// in each of these confs for messages still to be send or received. When the count goes
/// to 0, the entry is removed. When `receiving_exchange_confs` is empty, the test scheduler
/// is informed with a notification. When completion notification are received, the test ends.
fn multithread_stress_test() {
    const N_DISPATCHER: usize = 10;
    const N_STRESS_TESTER: usize = 100;
    const N_MESSAGES: usize = 100_000;
    const SEED: u64 = 0;

    #[derive(Clone)]
    struct ExchangeConf {
        pub peer: Addr,
        pub prng: oorandom::Rand32,
        pub count: usize,
    }

    struct StressTester {
        termination_observer: Addr,
        prng: oorandom::Rand32,
        emitting_exchange_confs: Vec<ExchangeConf>,
        receiving_exchange_confs: BTreeMap<Addr, ExchangeConf>,
        expected_request_id: RequestId,
        expected_response_src: Addr,
        expected_response_u32: u32,
    }

    enum Request {
        AddEmittingExchangeConf(ExchangeConf),
        AddReceivingExchangeConf(ExchangeConf),
        TestReq(u32 /*random data*/),
    }

    enum Response {
        AddEmittingExchangeConf(),
        AddReceivingExchangeConf(),
        TestReq((Addr /* source */, u32 /* random data */)),
    }

    enum Notification {
        StartTest(Addr /*termination observer*/),
        TestNotif((Addr /* source */, u32 /* random data */)),
        ProcessNext(),
        ExchangesCompleted(),
    }

    impl StressTester {
        pub fn new(termination_observer: &Addr, seed: u64) -> StressTester {
            StressTester {
                termination_observer: termination_observer.clone(),
                prng: oorandom::Rand32::new(seed),
                emitting_exchange_confs: Vec::new(),
                receiving_exchange_confs: BTreeMap::new(),
                expected_request_id: RequestId::MAX,
                expected_response_src: Addr::new(),
                expected_response_u32: u32::MAX,
            }
        }
    }

    impl StressTester {
        fn process_request(&mut self, context: &mut ProcessContext, request: &rtactor::Request) {
            match request.data.downcast_ref::<Request>().unwrap() {
                Request::AddEmittingExchangeConf(add_conf) => {
                    match self
                        .emitting_exchange_confs
                        .iter()
                        .position(|c| c.peer == add_conf.peer)
                    {
                        Some(index) => {
                            let current_conf = self.emitting_exchange_confs.get_mut(index).unwrap();
                            current_conf.prng = add_conf.prng;
                            current_conf.count += add_conf.count;
                        }
                        None => self.emitting_exchange_confs.push(add_conf.clone()),
                    }
                    context.send_response(request, Response::AddEmittingExchangeConf());
                }
                Request::AddReceivingExchangeConf(add_conf) => {
                    match self.receiving_exchange_confs.get_mut(&add_conf.peer) {
                        Some(current_conf) => {
                            current_conf.prng = add_conf.prng;
                            current_conf.count += add_conf.count;
                        }
                        None => {
                            self.receiving_exchange_confs
                                .insert(add_conf.peer.clone(), add_conf.clone());
                        }
                    };
                    context.send_response(request, Response::AddReceivingExchangeConf());
                }
                Request::TestReq(val) => {
                    // Receive a request and respond to it.
                    match self.receiving_exchange_confs.get_mut(&request.src) {
                        Some(conf) => {
                            assert_eq!(*val, conf.prng.rand_u32());
                            context.send_response(
                                request,
                                Response::TestReq((context.own_addr(), conf.prng.rand_u32())),
                            );
                            conf.count -= 1;
                            if conf.count == 0 {
                                self.receiving_exchange_confs.remove(&request.src).unwrap();
                            }
                            if self.receiving_exchange_confs.is_empty() {
                                // When nothing more to receive announce completion.
                                context
                                    .send_notification(
                                        &self.termination_observer,
                                        Notification::ExchangesCompleted(),
                                    )
                                    .unwrap();
                            }
                        }
                        None => {
                            panic!();
                        }
                    }
                }
            }
        }

        fn process_notification(
            &mut self,
            context: &mut ProcessContext,
            notif: &rtactor::Notification,
        ) {
            match notif.data.downcast_ref::<Notification>().unwrap() {
                Notification::StartTest(termination_observer) => {
                    self.termination_observer = termination_observer.clone();

                    if !self.emitting_exchange_confs.is_empty() {
                        context
                            .send_self_notification(Notification::ProcessNext())
                            .unwrap();
                    } else {
                        context
                            .send_notification(
                                &self.termination_observer,
                                Notification::ExchangesCompleted(),
                            )
                            .unwrap();
                    }
                }
                Notification::TestNotif((src, val)) => {
                    // Receive a test notification and do the checks.
                    let conf = self.receiving_exchange_confs.get_mut(src).unwrap();

                    assert!(*val == conf.prng.rand_u32());
                    conf.count -= 1;
                    if conf.count == 0 {
                        self.receiving_exchange_confs.remove(src).unwrap();
                    }
                    if self.receiving_exchange_confs.is_empty() {
                        // When nothing more to receive announce completion.
                        context
                            .send_notification(
                                &self.termination_observer,
                                Notification::ExchangesCompleted(),
                            )
                            .unwrap();
                    }
                }
                Notification::ProcessNext() => {
                    let rand_index = self
                        .prng
                        .rand_range(0..(self.emitting_exchange_confs.len() as u32))
                        as usize;
                    let conf = &mut self.emitting_exchange_confs[rand_index];

                    if self.prng.rand_u32() % 2 == 0 {
                        // Send a Request.
                        self.expected_request_id = context
                            .send_request(&conf.peer, Request::TestReq(conf.prng.rand_u32()));
                        self.expected_response_u32 = conf.prng.rand_u32();
                        self.expected_response_src = conf.peer.clone();

                        conf.count -= 1;
                        if conf.count == 0 {
                            self.emitting_exchange_confs.remove(rand_index);
                        }

                        // The continuation will be handled when the response is received.
                    } else {
                        // Send a notification.
                        context
                            .send_notification(
                                &conf.peer,
                                Notification::TestNotif((context.own_addr(), conf.prng.rand_u32())),
                            )
                            .unwrap();

                        conf.count -= 1;
                        if conf.count == 0 {
                            self.emitting_exchange_confs.remove(rand_index);
                        }

                        if !self.emitting_exchange_confs.is_empty() {
                            context
                                .send_notification(&context.own_addr(), Notification::ProcessNext())
                                .unwrap();
                        }
                    }
                }
                _ => panic!(),
            }
        }
    }

    impl Behavior for StressTester {
        fn process_message(&mut self, context: &mut ProcessContext, msg: &Message) {
            match msg {
                Message::Request(request) => self.process_request(context, request),
                Message::Response(response) => match &response.result {
                    Ok(data) => match data.downcast_ref::<Response>().unwrap() {
                        Response::TestReq((src, val)) => {
                            assert!(*src == self.expected_response_src);
                            assert!(*val == self.expected_response_u32);
                            assert!(response.request_id == self.expected_request_id);
                            assert!(response.id_eq(self.expected_request_id));

                            if !self.emitting_exchange_confs.is_empty() {
                                let_assert!(
                                    Ok(()) =
                                        context.send_self_notification(Notification::ProcessNext())
                                );
                            }
                        }
                        _ => panic!(),
                    },
                    Err(_) => panic!(),
                },
                Message::Notification(notif) => self.process_notification(context, notif),
            }
        }
    }

    let mut disp_addrs = Vec::<Addr>::new();
    let mut all_reactive_addrs = Vec::<Addr>::new();
    let mut remaining_stress_tester = N_STRESS_TESTER;
    let mut join_handles = Vec::<JoinHandle<()>>::new();
    let mut prng = oorandom::Rand32::new(SEED);
    let mut ordonnancer = ActiveMailbox::new(N_STRESS_TESTER);

    // Start a number of dispatchers.
    for i_disp in 0..N_DISPATCHER {
        let remaining_disp_count = N_DISPATCHER - i_disp;
        let avg_reactive_count = remaining_stress_tester / remaining_disp_count;
        let reactive_count = prng.rand_range(1..(max(avg_reactive_count, 2) as u32)) as usize;
        let seed = prng.rand_u32() as u64;
        let ordonnancer_addr = ordonnancer.addr().clone();
        let (disp_addr, join_handle, mut reactive_addrs) = spawn_dispatcher(1000, move |disp| {
            let mut reactives = Vec::<Addr>::with_capacity(reactive_count);
            let mut local_prng = oorandom::Rand32::new(seed);
            // In each dispatcher, start a number of reactives.
            for _i_reactive in 0..reactive_count {
                let addr = disp.register_reactive(Box::new(StressTester::new(
                    &ordonnancer_addr,
                    local_prng.rand_u32() as u64,
                )));
                reactives.push(addr);
            }
            reactives
        });
        remaining_stress_tester -= reactive_addrs.len();

        disp_addrs.push(disp_addr);
        join_handles.push(join_handle);
        all_reactive_addrs.append(&mut reactive_addrs);
    }

    // Instruct the reactives the messages they will send.
    let mut remaining_message_count = N_MESSAGES;
    while remaining_message_count > 0 {
        let rand_index_src = prng.rand_range(0..(all_reactive_addrs.len() as u32)) as usize;
        let rand_index_dst = prng.rand_range(0..(all_reactive_addrs.len() as u32)) as usize;
        let avg_count = max(remaining_message_count / all_reactive_addrs.len(), 2);

        let count = prng.rand_range(1..(avg_count as u32)) as usize;

        let mut conf = ExchangeConf {
            peer: all_reactive_addrs[rand_index_dst].clone(),
            prng: oorandom::Rand32::new(prng.rand_u32() as u64),
            count,
        };

        match ordonnancer
            .request_for(
                &all_reactive_addrs[rand_index_src],
                Request::AddEmittingExchangeConf(conf.clone()),
                TEST_TIMEOUT,
            )
            .unwrap()
        {
            Response::AddEmittingExchangeConf() => (),
            _ => panic!(),
        }

        conf.peer = all_reactive_addrs[rand_index_src].clone();
        match ordonnancer
            .request_for(
                &all_reactive_addrs[rand_index_dst],
                Request::AddReceivingExchangeConf(conf),
                TEST_TIMEOUT,
            )
            .unwrap()
        {
            Response::AddReceivingExchangeConf() => (),
            _ => panic!(),
        }

        remaining_message_count -= count;
    }

    // Start the reactives operations.
    for reactive in &all_reactive_addrs {
        send_notification(reactive, Notification::StartTest(ordonnancer.addr())).unwrap();
    }

    // Wait for the reactives to finish the operations.
    for _ in &all_reactive_addrs {
        match ordonnancer.wait_message_for(TEST_TIMEOUT).unwrap() {
            Message::Notification(notif) => {
                match notif.data.downcast_ref::<Notification>().unwrap() {
                    Notification::ExchangesCompleted() => (),
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    // Request all dispatchers to stop.
    for disp_addr in &disp_addrs {
        if let dispatcher::Response::StopDispatcher() = ordonnancer
            .request_for(
                disp_addr,
                dispatcher::Request::StopDispatcher {},
                TEST_TIMEOUT,
            )
            .unwrap()
        {
        } else {
            panic!();
        }
    }

    // Wait all dispatcher threads to terminate.
    while !join_handles.is_empty() {
        join_handles.pop().unwrap().join().unwrap();
    }
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
                            if i != 2 {
                                assert!(!self.timers[i].is_scheduling(notif));
                            } else {
                                assert!(self.timers[i].is_scheduling(notif))
                            }
                        }
                    } else if self.timers[0].is_scheduling(notif) {
                        assert!(self.index == 1);
                        assert!(
                            context.now().saturating_sub(&self.begin) >= Duration::from_millis(5)
                        );
                        self.index += 1;
                    } else if self.timers[1].is_scheduling(notif) {
                        assert!(self.index == 2);
                        assert!(
                            context.now().saturating_sub(&self.begin) >= Duration::from_millis(6)
                        );
                        context.unschedule(&mut self.timers[3]);
                        self.index += 1;
                    } else if self.timers[4].is_scheduling(notif) {
                        assert!(self.index == 3);
                        assert!(
                            context.now().saturating_sub(&self.begin) >= Duration::from_millis(230)
                        );
                        context.schedule_for(&mut self.timers[3], Duration::from_millis(42));
                        self.index += 1;
                    } else if self.timers[3].is_scheduling(notif) {
                        assert!(self.index == 4);
                        assert!(
                            context.now().saturating_sub(&self.begin)
                                >= Duration::from_millis(230 + 42)
                        );
                        self.index += 1;
                        context
                            .send_notification(&self.observer, Notification::Finished())
                            .unwrap();
                    }
                }
            }
        }
    }
    let mut observer = ActiveMailbox::new(1);
    let observer_addr = observer.addr();

    // Start a dispatcher inside its own thread.
    // The active object is created with the closure called inside the dispatcher thread.
    // This allows to have reactive object that are not movable between threads.
    let (dispatcher_addr, join_handle, _) = spawn_dispatcher(10, move |disp| {
        // Create a reactive object on the heap.
        let test_reactive = Box::new(Tester::new(&observer_addr));

        // Move it inside the dispatcher and return the reactive address as the return of `setup_func`
        send_notification(
            &disp.register_reactive(test_reactive),
            Notification::Start(),
        )
        .unwrap();
    });

    let_assert!(Ok(Message::Notification(notif)) = observer.wait_message_for(TEST_TIMEOUT));
    let_assert!(Some(Notification::Finished()) = notif.data.downcast_ref());

    let_assert!(
        Ok(dispatcher::Response::StopDispatcher()) = observer.request_for(
            &dispatcher_addr,
            dispatcher::Request::StopDispatcher {},
            TEST_TIMEOUT,
        )
    );

    join_handle.join().unwrap();
}

#[test]
fn test_sync_access() {
    let (disp_addr, join_handle, _) = spawn_dispatcher(1, |_disp| ());

    let mut dispatcher = dispatcher::SyncAccessor::new(&disp_addr);

    assert!(
        dispatcher
            .execute_fn(
                |_disp| {
                    assert!(_disp.addr().is_valid());
                    456_i32
                },
                TEST_TIMEOUT,
            )
            .unwrap()
            == 456
    );

    dispatcher.stop_dispatcher(TEST_TIMEOUT).unwrap();

    join_handle.join().unwrap();
}
