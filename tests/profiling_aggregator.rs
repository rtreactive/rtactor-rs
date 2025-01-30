#[cfg(feature = "mockall")]
mod with_mockall {

    use rtactor as rt;

    use assert2::let_assert;
    use log::trace;
    use rstest::rstest;
    use rt::simulation::ReactiveMocker;
    use rt::{
        profiled_actor::{self, AvgMetric, Metric},
        profiling_aggregator::{
            self, ActorSelection, ProfilingAggregator, SyncNotifier, SyncRequester,
        },
    };
    use std::{cell::RefCell, time::Duration};

    fn init_log() {
        use std::sync::Once;
        static UNIQUE_INIT: Once = Once::new();
        UNIQUE_INIT.call_once(|| {
            env_logger::builder()
                .is_test(true)
                .format_timestamp(None)
                .filter_level(log::LevelFilter::Off)
                .init();
        });
    }

    struct ProfiledData {
        avg_metric: AvgMetric,
    }

    struct Fixture {
        pub disp: RefCell<rt::simulation::SimulationDispatcher>,

        pub observer_mockers: Vec<ReactiveMocker<()>>,
        pub profiled_mockers: Vec<ReactiveMocker<ProfiledData>>,
        pub aggregator_addr: rt::Addr,
        pub seq: mockall::Sequence,
    }

    impl Fixture {
        pub fn aggregator_accessor(&self) -> profiling_aggregator::SimSyncAccessor {
            profiling_aggregator::SimSyncAccessor::new(&self.disp, &self.aggregator_addr)
        }

        pub fn checkpoint(&self) {
            for observer_mocker in &self.observer_mockers {
                observer_mocker.mock().checkpoint();
            }
            for profiled_mocker in &self.profiled_mockers {
                profiled_mocker.mock().checkpoint();
            }
        }
    }

    #[rstest::fixture]
    fn fix() -> Fixture {
        init_log();
        let disp = RefCell::new(rt::simulation::SimulationDispatcher::new(10));

        let mut observer_mockers = Vec::new();
        for _i in 0..3 {
            observer_mockers.push(ReactiveMocker::new(&disp, ()));
        }

        let mut profiled_mockers = Vec::new();
        for _i in 0..3 {
            profiled_mockers.push(ReactiveMocker::new(
                &disp,
                ProfiledData {
                    avg_metric: AvgMetric::new("val_avg"),
                },
            ));
        }

        let aggregator_addr = disp
            .borrow_mut()
            .register_reactive(Box::new(ProfilingAggregator::new()));

        Fixture {
            disp,
            observer_mockers,
            profiled_mockers,
            aggregator_addr,
            seq: mockall::Sequence::new(),
        }
    }

    #[rstest]
    fn test_registration(fix: Fixture) {
        for _i in 0..3 {
            for _j in 0..2 {
                fix.aggregator_accessor()
                    .register_observer(fix.observer_mockers[0].addr(), Duration::ZERO)
                    .unwrap();
            }

            for _j in 0..2 {
                fix.aggregator_accessor()
                    .unregister_observer(fix.observer_mockers[0].addr(), Duration::ZERO)
                    .unwrap();
            }

            fix.aggregator_accessor()
                .register_profiled_actors(
                    vec![
                        ("actor_1".to_string(), fix.profiled_mockers[0].addr()),
                        ("actor_2".to_string(), fix.profiled_mockers[0].addr()),
                    ],
                    Duration::ZERO,
                )
                .unwrap();

            fix.aggregator_accessor()
                .unregister_profiled_actors(ActorSelection::All, Duration::ZERO)
                .unwrap();
        }

        fix.aggregator_accessor()
            .register_profiled_actors(
                vec![
                    ("actor_1".to_string(), fix.profiled_mockers[0].addr()),
                    ("actor_2".to_string(), fix.profiled_mockers[1].addr()),
                ],
                Duration::ZERO,
            )
            .unwrap();

        fix.aggregator_accessor()
            .unregister_profiled_actors(
                ActorSelection::SelectedNames(vec![
                    "actor_1".to_string(),
                    "unknown_is_valid".to_string(),
                ]),
                Duration::ZERO,
            )
            .unwrap();

        fix.aggregator_accessor()
            .unregister_profiled_actors(
                ActorSelection::SelectedAddresses(vec![
                    fix.profiled_mockers[1].addr(),
                    rt::Addr::INVALID,
                ]),
                Duration::ZERO,
            )
            .unwrap();
    }

    #[rstest]
    fn test_simple(mut fix: Fixture) {
        for i in 0..3 {
            trace!("loop {}", i);

            fix.profiled_mockers[0]
                .mock()
                .expect_process_request()
                .once()
                .in_sequence(&mut fix.seq)
                .returning(move |data, context, request| {
                    data.avg_metric.records(23.4);

                    assert!(profiled_actor::try_process_request(
                        context,
                        request,
                        |list| {
                            data.avg_metric.snapshot_in(list);
                        }
                    ));
                });

            let i_for_observer = i;
            fix.observer_mockers[0]
                .mock()
                .expect_process_notification()
                .once()
                .in_sequence(&mut fix.seq)
                .returning(move |_, context, notification| {
                    let_assert!(
                        Some(profiling_aggregator::observer::Notification::MetricUpdate {
                            timestamp,
                            names_metric_lists
                        }) = notification.data.downcast_ref()
                    );

                    assert_eq!(names_metric_lists.len(), 1);
                    assert_eq!(context.now().saturating_sub(timestamp).as_micros(), 0);

                    let (name, list) = &names_metric_lists[0];
                    assert_eq!(*name, format!("actor0_loop{}", i_for_observer));
                    assert_eq!(list.len(), 1);
                    assert_eq!(list[0].name, "val_avg");
                    assert_eq!(list[0].value, 23.4);
                });

            // ---------- registration ---------------
            fix.aggregator_accessor()
                .register_observer(fix.observer_mockers[0].addr(), Duration::ZERO)
                .unwrap();

            fix.aggregator_accessor()
                .register_profiled_actors(
                    vec![(
                        format!("actor0_loop{}", i).to_string(),
                        fix.profiled_mockers[0].addr(),
                    )],
                    Duration::ZERO,
                )
                .unwrap();

            // -------------- ask ---------------------

            fix.aggregator_accessor()
                .snapshot_metrics(ActorSelection::All)
                .unwrap();

            fix.disp.borrow_mut().process_for(Duration::ZERO);

            // ---------- unregistration ---------------

            fix.aggregator_accessor()
                .unregister_observer(fix.observer_mockers[0].addr(), Duration::ZERO)
                .unwrap();

            fix.aggregator_accessor()
                .unregister_profiled_actors(
                    ActorSelection::SelectedNames(vec![format!("actor0_loop{}", i).to_string()]),
                    Duration::ZERO,
                )
                .unwrap();

            fix.checkpoint();
        }
    }

    #[rstest]
    fn test_multiple(mut fix: Fixture) {
        for i in 0..3 {
            trace!("i={}", i);

            for profiled_mocker in &fix.profiled_mockers {
                profiled_mocker
                    .mock()
                    .expect_process_request()
                    .once()
                    .in_sequence(&mut fix.seq)
                    .returning(move |data, context, request| {
                        data.avg_metric.records(23.4);

                        assert!(profiled_actor::try_process_request(
                            context,
                            request,
                            |list| {
                                data.avg_metric.snapshot_in(list);
                            }
                        ));
                    });
            }

            let i_for_observer = i;
            for observer_mocker in &fix.observer_mockers {
                observer_mocker
                    .mock()
                    .expect_process_notification()
                    .once()
                    .in_sequence(&mut fix.seq)
                    .returning(move |_, context, notification| {
                        let_assert!(
                            Some(profiling_aggregator::observer::Notification::MetricUpdate {
                                timestamp,
                                names_metric_lists
                            }) = notification.data.downcast_ref()
                        );

                        assert_eq!(names_metric_lists.len(), 3);
                        assert_eq!(context.now().saturating_sub(timestamp).as_micros(), 0);

                        let mut j = 0;
                        for (name, list) in names_metric_lists {
                            assert_eq!(*name, format!("loop{}_profiled{}", i_for_observer, j));
                            assert_eq!(list.len(), 1);
                            assert_eq!(list[0].name, "val_avg");
                            assert_eq!(list[0].value, 23.4);
                            j += 1;
                        }
                    });
            }

            // ---------- registration ---------------
            for observer_mocker in &fix.observer_mockers {
                fix.aggregator_accessor()
                    .register_observer(observer_mocker.addr(), Duration::ZERO)
                    .unwrap();
            }

            for j in 0..fix.profiled_mockers.len() {
                fix.aggregator_accessor()
                    .register_profiled_actors(
                        vec![(
                            format!("loop{}_profiled{}", i, j).to_string(),
                            fix.profiled_mockers[j].addr(),
                        )],
                        Duration::ZERO,
                    )
                    .unwrap();
            }

            // -------------- ask ---------------------

            fix.aggregator_accessor()
                .snapshot_metrics(ActorSelection::All)
                .unwrap();

            fix.disp.borrow_mut().process_for(Duration::ZERO);

            // ---------- unregistration ---------------

            for observer_mocker in &fix.observer_mockers {
                fix.aggregator_accessor()
                    .unregister_observer(observer_mocker.addr(), Duration::ZERO)
                    .unwrap();
            }

            fix.aggregator_accessor()
                .unregister_profiled_actors(ActorSelection::All, Duration::ZERO)
                .unwrap();

            fix.checkpoint();
        }
    }
}
