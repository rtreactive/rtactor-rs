/// Example code provided in the top level README.md.
use std::{cell::RefCell, time::Duration};

use rtactor as rt;

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

    const BROADCAST_PERIOD: Duration = Duration::from_millis(500);
    const CAN_ACCELERATION_ID: u32 = 0x1000;

    impl Broadcaster {
        pub fn new() -> Broadcaster {
            Broadcaster {
                can_controller_addr: rt::Addr::INVALID,
                sample_count: 0,
                acceleration_sum: [0; 3],
                state: State::Stopped,
                timer: rt::Timer::new(),
            }
        }

        fn reset_sum(&mut self) {
            for a in &mut self.acceleration_sum {
                *a = 0;
            }
            self.sample_count = 0;
        }
    }

    impl Default for Broadcaster {
        fn default() -> Self {
            Self::new()
        }
    }

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

    impl Broadcaster {
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
    }
}

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
