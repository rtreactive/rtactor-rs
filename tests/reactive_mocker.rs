#[cfg(feature = "mockall")]
mod has_mockall {

    use std::{cell::RefCell, time::Duration};

    use assert2::let_assert;
    use rt::simulation::{ReactiveMocker, SimulationDispatcher};
    use rtactor as rt;

    use rtactor::define_sim_sync_accessor;
    use rtactor_macros::{ResponseEnum, SyncNotifier, SyncRequester};

    #[derive(ResponseEnum, SyncRequester)]
    enum Request {
        SendAString { s: String },
    }

    #[derive(SyncNotifier)]
    enum Notification {
        NotifyAnI32 { i: i32 },
    }

    define_sim_sync_accessor!(SimSyncAccessor, SyncRequester, SyncNotifier);

    struct Data {
        a_data: f32,
    }

    impl Data {
        pub fn new() -> Self {
            Self { a_data: 3.4 }
        }
    }

    #[test]
    fn test_mocker_empty() {
        let disp = RefCell::new(SimulationDispatcher::new(10));
        let mocker = ReactiveMocker::new(&disp, Data::new());
        assert!(mocker.addr().is_valid());
        assert_eq!(mocker.data().a_data, 3.4);

        mocker.data().a_data = -53.2;
        assert_eq!(mocker.data().a_data, -53.2);

        disp.borrow_mut().process_for(Duration::from_secs(1));
        assert_eq!(mocker.data().a_data, -53.2);
    }

    #[test]
    fn test_mocker_simple() {
        let disp = RefCell::new(SimulationDispatcher::new(10));
        let mocker = ReactiveMocker::new(&disp, Data::new());

        assert!(mocker.addr().is_valid());

        mocker
            .mock()
            .expect_process_request()
            .returning(|data, context, request| {
                let_assert!(Some(Request::SendAString { s }) = request.data.downcast_ref());
                assert_eq!(s, "a string");
                assert_eq!(data.a_data, 3.4);

                context.send_response(request, Response::SendAString());
            });

        let mut accessor = SimSyncAccessor::new(&disp, &mocker.addr());

        accessor
            .send_a_string("a string".to_string(), Duration::ZERO)
            .unwrap();

        mocker.mock().checkpoint();

        mocker
            .mock()
            .expect_process_notification()
            .returning(|data, _context, notification| {
                let_assert!(
                    Some(Notification::NotifyAnI32 { i }) = notification.data.downcast_ref()
                );
                assert_eq!(*i, 678);
                data.a_data = 67.3;
            });

        rt::send_notification(&mocker.addr(), Notification::NotifyAnI32 { i: 678 }).unwrap();

        disp.borrow_mut().process_for(Duration::ZERO);

        assert_eq!(mocker.data().a_data, 67.3);

        mocker.mock().checkpoint();
    }
}
