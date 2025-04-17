//! An interface for actor profiled by profiling_aggregator and the corresponding Metrics struct.
//!
//! Different Metric can be used and are interchangeable.
//!
//! Use try_process_request() to implement profiled_actor in your Behavior.

use crate as rt;

use rt::{
    define_sim_sync_accessor, define_sync_accessor,
    rtactor_macros::{ResponseEnum, SyncRequester},
};
use std::cell::Cell;

/// Helper function to gives the metrics
///
/// Return true if a request was processed.
pub fn try_process_request<F>(
    context: &mut rt::ProcessContext,
    request: &rt::Request,
    mut func: F,
) -> bool
where
    F: FnMut(&mut Vec<MetricEntry>),
{
    if let Some(req_data) = request.data.downcast_ref() {
        match req_data {
            Request::GetMetrics { metric_list } => {
                if let Some(mut list) = metric_list.replace(None) {
                    func(&mut list);

                    context.send_response(request, Response::GetMetrics(Cell::new(Some(list))));
                }
            }
        }
        true
    } else {
        false
    }
}

// Internal, not very useful if try_process_request is used.
#[derive(ResponseEnum, SyncRequester)]
pub enum Request {
    // The cell allows to avoid copy of the vector.
    #[response_val(Cell<Option<Vec<MetricEntry>>>)]
    GetMetrics {
        metric_list: Cell<Option<Vec<MetricEntry>>>,
    },
}

define_sim_sync_accessor!(SimSyncAccessor, SyncRequester);
define_sync_accessor!(SyncAccessor, SyncRequester);

#[derive(Clone, Debug)]
pub struct MetricEntry {
    pub name: &'static str,
    pub value: f64,
}

pub trait Metric {
    fn snapshot(&mut self) -> MetricEntry;
    fn snapshot_in(&mut self, list: &mut Vec<MetricEntry>) {
        list.push(self.snapshot());
    }
    fn records(&mut self, val: f64);

    /// Take now() from the context.
    /// Using this allows DeactivatedMetric to skip this costly operation.
    fn take_begin_instant(&self, context: &rt::ProcessContext) -> rt::Instant {
        context.now()
    }

    /// Record a duration, take_begin_instant() should be used for begin_instant.
    fn records_duration(&mut self, context: &rt::ProcessContext, begin_instant: rt::Instant) {
        self.records(context.now().saturating_sub(&begin_instant).as_secs_f64())
    }
}

/// Record the minimum or INFINITY if records() never called.
pub struct MinMetric {
    pub name: &'static str,
    pub value: f64,
}

impl MinMetric {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            value: f64::INFINITY,
        }
    }
}

impl Metric for MinMetric {
    fn snapshot(&mut self) -> MetricEntry {
        let val = self.value;
        self.value = f64::INFINITY;
        MetricEntry {
            name: self.name,
            value: val,
        }
    }
    fn records(&mut self, val: f64) {
        self.value = self.value.min(val);
    }
}

/// Record the minimum or -INFINITY if records() never called.
pub struct MaxMetric {
    pub name: &'static str,
    pub value: f64,
}

impl MaxMetric {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            value: -f64::INFINITY,
        }
    }
}

impl Metric for MaxMetric {
    fn snapshot(&mut self) -> MetricEntry {
        let val = self.value;
        self.value = -f64::INFINITY;
        MetricEntry {
            name: self.name,
            value: val,
        }
    }
    fn records(&mut self, val: f64) {
        self.value = self.value.max(val);
    }
}

pub struct AvgMetric {
    pub name: &'static str,
    pub sum: f64,
    pub count: u32,
}

/// A metric that does the average between snapshots.
impl AvgMetric {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            sum: 0.0,
            count: 0,
        }
    }

    /// Return the number of time records was done
    pub fn count(&self) -> u32 {
        self.count
    }
}

impl Metric for AvgMetric {
    fn snapshot(&mut self) -> MetricEntry {
        let avg = if self.count < u32::MAX {
            self.sum / self.count as f64
        } else {
            f64::NAN
        };

        self.count = 0;
        self.sum = 0.0;

        MetricEntry {
            name: self.name,
            value: avg,
        }
    }

    fn records(&mut self, val: f64) {
        self.count = self.count.saturating_add(1);
        self.sum += val;
    }
}

/// A metric that computes the Root Mean Square between snapshots.
pub struct RmsMetric {
    pub name: &'static str,
    pub sqr_sum: f64,
    pub count: u32,
}

impl RmsMetric {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            sqr_sum: 0.0,
            count: 0,
        }
    }

    /// Return the number of time records() was called.
    pub fn count(&self) -> u32 {
        self.count
    }
}

impl Metric for RmsMetric {
    fn snapshot(&mut self) -> MetricEntry {
        let rms = if self.count < u32::MAX {
            (self.sqr_sum / (self.count as f64)).sqrt()
        } else {
            f64::NAN
        };

        self.count = 0;
        self.sqr_sum = 0.0;

        MetricEntry {
            name: self.name,
            value: rms,
        }
    }

    fn records(&mut self, val: f64) {
        self.count = self.count.saturating_add(1);
        self.sqr_sum += val.powi(2);
    }
}

/// A metric that do not generate any code but is a drop in replacement to other Metric.
pub struct DeactivatedMetric();

impl DeactivatedMetric {
    pub fn new(_name: &'static str) -> Self {
        Self()
    }
}

impl Metric for DeactivatedMetric {
    fn snapshot(&mut self) -> MetricEntry {
        MetricEntry {
            name: "dummy_metric",
            value: f64::NAN,
        }
    }

    fn records(&mut self, _val: f64) {}

    fn take_begin_instant(&self, _context: &rt::ProcessContext) -> rt::Instant {
        rt::Instant::INFINITY
    }

    fn records_duration(&mut self, _context: &rt::ProcessContext, _begin_instant: rt::Instant) {}

    fn snapshot_in(&mut self, _list: &mut Vec<MetricEntry>) {}
}
