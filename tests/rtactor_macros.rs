use std::time::Duration;

use assert2::let_assert;
use rtactor::rtactor_macros::{ResponseEnum, SyncNotifier, SyncRequester};
use rtactor::simulation::SimulationDispatcher;
use rtactor::Error;
use rtactor::{self, define_sim_sync_accessor, define_sync_accessor, dispatcher};

/// Test that proc_macros in rtactor_macros crate works (can not be tested in the crate).
#[test]
fn sync_macros() {
    #[derive(SyncRequester, ResponseEnum)]
    pub enum Request {
        WithoutArgs {},

        With1ArgIn0Out {
            arg1: f32,
        },

        #[response_val(u8)]
        With0ArgIn1Out {},

        #[response_val((Result<i8, rtactor::Error>, i32))]
        With2ArgsIn2Out {
            arg1: bool,
            arg2: Option<i32>,
        },
    }

    #[derive(SyncNotifier)]
    pub enum Notification {
        NotifWithoutArgs {},
        NotifWith1Arg { arg1: u64 },
        NotifWith2Args { arg1: String, arg2: char },
    }

    struct TestReactive {}
    impl rtactor::Behavior for TestReactive {
        fn process_message<'a>(
            &mut self,
            context: &'a mut rtactor::ProcessContext,
            msg: &rtactor::Message,
        ) {
            match msg {
                rtactor::Message::Request(request) => {
                    if let Some(req) = request.data.downcast_ref::<Request>() {
                        match req {
                            Request::WithoutArgs {} => {
                                context.send_response(request, Response::WithoutArgs());
                            }
                            Request::With1ArgIn0Out { arg1 } => {
                                assert!(arg1 == &3.45f32);
                                context.send_response(request, Response::With1ArgIn0Out());
                            }
                            Request::With0ArgIn1Out {} => {
                                context.send_response(request, Response::With0ArgIn1Out(128));
                            }
                            Request::With2ArgsIn2Out { arg1, arg2 } => {
                                assert!(!arg1);
                                if let Some(arg2_val) = arg2 {
                                    context.send_response(
                                        request,
                                        Response::With2ArgsIn2Out((Ok(34), *arg2_val)),
                                    );
                                } else {
                                    panic!();
                                }
                            }
                        }
                    }
                }
                rtactor::Message::Response(_) => panic!(),
                rtactor::Message::Notification(notification) => {
                    if let Some(notif) = notification.data.downcast_ref::<Notification>() {
                        match notif {
                            Notification::NotifWithoutArgs {} => (),
                            Notification::NotifWith1Arg { arg1 } => {
                                assert!(*arg1 == 333);
                            }
                            Notification::NotifWith2Args { arg1, arg2 } => {
                                assert!(arg1 == "foobar");
                                assert!(*arg2 == 'b');
                            }
                        }
                    }
                }
            }
        }
    }

    // ------------ Sim macro -------------
    {
        let disp = std::cell::RefCell::new(SimulationDispatcher::new(1));
        let test_reactive_addr = disp
            .borrow_mut()
            .register_reactive(Box::new(TestReactive {}));

        define_sim_sync_accessor!(TestReactiveSyncAccessor, SyncRequester, SyncNotifier);

        let mut accessor = TestReactiveSyncAccessor::new(&disp, &test_reactive_addr);

        assert!(accessor.target_addr() == &test_reactive_addr);

        let timeout = Duration::from_secs(10);
        accessor.without_args(timeout).unwrap();
        accessor.with_1_arg_in_0_out(3.45f32, timeout).unwrap();
        assert!(accessor.with_0_arg_in_1_out(timeout).unwrap() == 128);
        if let (Ok(arg1_val), arg2) = accessor
            .with_2_args_in_2_out(false, Some(-987), timeout)
            .unwrap()
        {
            assert!(arg1_val == 34);
            assert!(arg2 == -987);
        };

        accessor.notif_without_args().unwrap();
        disp.borrow_mut().process_for(Duration::ZERO);

        accessor.notif_with_1_arg(333).unwrap();
        disp.borrow_mut().process_for(Duration::ZERO);

        accessor
            .notif_with_2_args("foobar".to_string(), 'b')
            .unwrap();
        disp.borrow_mut().process_for(Duration::ZERO);
    }

    // ------------ real thread ------------------
    {
        let (disp_addr, join_handle, test_reactive_addr) =
            rtactor::spawn_dispatcher(10, |disp| disp.register_reactive(Box::new(TestReactive {})));

        define_sync_accessor!(TestReactiveSyncAccessor, SyncRequester, SyncNotifier);

        let mut accessor = TestReactiveSyncAccessor::new(&test_reactive_addr);

        assert!(accessor.target_addr() == &test_reactive_addr);

        let timeout = Duration::from_secs(3600);
        accessor.without_args(timeout).unwrap();
        accessor.with_1_arg_in_0_out(3.45f32, timeout).unwrap();
        assert!(accessor.with_0_arg_in_1_out(timeout).unwrap() == 128);
        if let (Ok(arg1_val), arg2) = accessor
            .with_2_args_in_2_out(false, Some(-987), timeout)
            .unwrap()
        {
            assert!(arg1_val == 34);
            assert!(arg2 == -987);
        };

        accessor.notif_without_args().unwrap();

        accessor.notif_with_1_arg(333).unwrap();

        accessor
            .notif_with_2_args("foobar".to_string(), 'b')
            .unwrap();

        let mut disp_accessor = dispatcher::SyncAccessor::new(&disp_addr);

        disp_accessor.stop_dispatcher(timeout).unwrap();
        join_handle.join().unwrap();
    }
}

#[test]
// Check that an invalid request_id fails
fn test_sim_sync_accessor_invalid_request_id() {
    #[derive(SyncRequester, ResponseEnum)]
    pub enum Request {
        WithoutArgs {},
    }

    struct TestReactive {}
    impl rtactor::Behavior for TestReactive {
        fn process_message<'a>(
            &mut self,
            context: &'a mut rtactor::ProcessContext,
            msg: &rtactor::Message,
        ) {
            match msg {
                rtactor::Message::Request(request) => {
                    if let Some(req) = request.data.downcast_ref::<Request>() {
                        match req {
                            Request::WithoutArgs {} => {
                                context.send_addr_id_response(
                                    &request.src,
                                    request.id + 4050,
                                    Response::WithoutArgs(),
                                );
                            }
                        }
                    }
                }
                _ => panic!(),
            }
        }
    }

    // ------------ Sim macro -------------
    {
        let disp = std::cell::RefCell::new(SimulationDispatcher::new(1));
        let test_reactive_addr = disp
            .borrow_mut()
            .register_reactive(Box::new(TestReactive {}));

        define_sim_sync_accessor!(TestReactiveSyncAccessor, SyncRequester);

        let mut accessor = TestReactiveSyncAccessor::new(&disp, &test_reactive_addr);

        // TODO this must be Error::Timeout to be semantically correct.
        let_assert!(Err(Error::NoMessage) = accessor.without_args(Duration::ZERO));
    }

    // ------------ real thread ------------------

    {
        let (disp_addr, join_handle, test_reactive_addr) =
            rtactor::spawn_dispatcher(10, |disp| disp.register_reactive(Box::new(TestReactive {})));

        define_sync_accessor!(TestReactiveSyncAccessor, SyncRequester);

        let mut accessor = TestReactiveSyncAccessor::new(&test_reactive_addr);

        let mut disp_accessor = dispatcher::SyncAccessor::new(&disp_addr);

        let timeout = Duration::from_secs(3600);

        let_assert!(Err(Error::Timeout) = accessor.without_args(Duration::from_millis(1000)));

        disp_accessor.stop_dispatcher(timeout).unwrap();
        join_handle.join().unwrap();
    }
}
