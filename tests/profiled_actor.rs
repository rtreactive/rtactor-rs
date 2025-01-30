use approx::assert_abs_diff_eq;
use rtactor as rt;

use assert2::let_assert;
use rt::profiled_actor::{
    self, AvgMetric, DeactivatedMetric, MaxMetric, Metric, MinMetric, RmsMetric, SyncRequester,
};
use std::{
    cell::{Cell, RefCell},
    f64::{INFINITY, NAN},
    time::Duration,
};

#[test]
fn test_min_metric() {
    let mut metric = MinMetric::new("min_buf_level");

    let mut list = Vec::with_capacity(3);

    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, INFINITY);
    assert_eq!(entry.name, "min_buf_level");

    metric.records(1.0);
    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, 1.0);
    assert_eq!(entry.name, "min_buf_level");

    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, INFINITY);
    assert_eq!(entry.name, "min_buf_level");

    metric.records(-3.);
    metric.records(NAN);
    metric.records(-7.5);
    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, -7.5);
    assert_eq!(entry.name, "min_buf_level");
}

#[test]
fn test_max_metric() {
    let mut metric = MaxMetric::new("max_buf_level");

    let mut list = Vec::with_capacity(3);

    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, -INFINITY);
    assert_eq!(entry.name, "max_buf_level");

    metric.records(3e6);
    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, 3e6);
    assert_eq!(entry.name, "max_buf_level");

    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, -INFINITY);
    assert_eq!(entry.name, "max_buf_level");

    metric.records(34.);
    metric.records(NAN);
    metric.records(5.);
    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, 34.);
    assert_eq!(entry.name, "max_buf_level");
}

#[test]
fn test_avg_metric() {
    let mut metric = AvgMetric::new("v_avg");

    let mut list = Vec::with_capacity(3);

    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert!(entry.value.is_nan());
    assert_eq!(entry.name, "v_avg");

    metric.records(-1e-3);
    metric.records(-2e-3);
    metric.records(-3e-3);
    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_eq!(entry.value, -2e-3);
    assert_eq!(entry.name, "v_avg");

    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert!(entry.value.is_nan());
    assert_eq!(entry.name, "v_avg");
}

#[test]
fn test_rms_metric() {
    let mut metric = RmsMetric::new("v_rms");

    let mut list = Vec::with_capacity(3);

    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert!(entry.value.is_nan());
    assert_eq!(entry.name, "v_rms");

    metric.records(-1e-3f64);
    metric.records(2e-3f64);
    metric.records(3e-3f64);
    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert_abs_diff_eq!(
        entry.value,
        (((-1e-3f64).powi(2) + 2e-3f64.powi(2) + 3e-3f64.powi(2)) / 3.0).sqrt(),
        epsilon = 1e-9
    );
    assert_eq!(entry.name, "v_rms");

    metric.snapshot_in(&mut list);

    let_assert!(Some(entry) = list.pop());
    assert!(entry.value.is_nan());
    assert_eq!(entry.name, "v_rms");
}

#[test]
fn test_dummy_metric() {
    let mut metric = DeactivatedMetric::new("dummy");

    let mut list = Vec::with_capacity(3);

    metric.snapshot_in(&mut list);

    assert!(list.pop().is_none());

    metric.records(3.4);

    metric.snapshot_in(&mut list);

    assert!(list.pop().is_none());
}

#[test]
fn test_take_duration() {
    struct Reactive {
        avg_duration: AvgMetric,
        deactivated_duration: DeactivatedMetric,
    }

    impl rt::Behavior for Reactive {
        fn process_message<'a, 'b>(
            &mut self,
            context: &'a mut rt::ProcessContext<'b>,
            msg: &rt::Message,
        ) {
            match msg {
                rt::Message::Request(request) => assert!(profiled_actor::try_process_request(
                    context,
                    request,
                    |list| { self.avg_duration.snapshot_in(list) }
                )),
                rt::Message::Response(_) => panic!(),
                rt::Message::Notification(_) => {
                    let begin = self.avg_duration.take_begin_instant(context);
                    let begin_dummy = self.avg_duration.take_begin_instant(context);
                    self.avg_duration.records_duration(context, begin);
                    self.deactivated_duration
                        .records_duration(context, begin_dummy);
                }
            }
        }
    }

    let disp = RefCell::new(rt::simulation::SimulationDispatcher::new(10));

    let mut accessor = profiled_actor::SimSyncAccessor::new(
        &disp,
        &disp.borrow_mut().register_reactive(Box::new(Reactive {
            avg_duration: AvgMetric::new("avg_dur"),
            deactivated_duration: DeactivatedMetric::new("deac_dur"),
        })),
    );

    let metrics = accessor
        .get_metrics(Cell::new(Some(Vec::new())), Duration::ZERO)
        .unwrap()
        .take()
        .unwrap();

    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].name, "avg_dur");
    assert!(metrics[0].value.is_nan());

    rt::send_notification(accessor.target_addr(), ()).unwrap();

    let metrics = accessor
        .get_metrics(Cell::new(Some(Vec::new())), Duration::ZERO)
        .unwrap()
        .take()
        .unwrap();

    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].name, "avg_dur");
    assert_eq!(metrics[0].value, 0.0);
}
