//! An actor were addresses of profiled_actor are registered and collect metrics when a snapshot is taken.
//!
//! self::observer actors can be registered and are notified at the end of the snapshot.
//!
//! The design is done so that the data allocation is done in the aggregator code,
//! so there is no reallocation in the profiled code (useful in the future when rtactor
//! message passing use memory pools).

use crate as rt;

use rt::profiled_actor::{self, MetricEntry};
use rt::{
    define_sim_sync_accessor, define_sync_accessor,
    rtactor_macros::{ResponseEnum, SyncNotifier, SyncRequester},
};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
};

// ---------- public actor interfaces -----------

#[derive(ResponseEnum, SyncRequester)]
pub enum Request {
    RegisterObserver {
        addr: rt::Addr,
    },
    UnregisterObserver {
        addr: rt::Addr,
    },

    RegisterProfiledActors {
        /// Pair of name and addresses that implement profiled_actor
        names_addresses: Vec<(String, rt::Addr)>,
    },
    UnregisterProfiledActors {
        selection: ActorSelection,
    },
}

define_sim_sync_accessor!(SimSyncAccessor, SyncRequester, SyncNotifier);
define_sync_accessor!(SyncAccessor, SyncRequester, SyncNotifier);

#[derive(SyncNotifier)]
pub enum Notification {
    SnapshotMetrics { selection: ActorSelection },
}

pub enum ActorSelection {
    All,
    SelectedNames(Vec<String>),
    SelectedAddresses(Vec<rt::Addr>),
}

pub mod observer {
    use rtactor::{define_sim_sync_accessor, define_sync_accessor, rtactor_macros::SyncNotifier};

    use crate::profiled_actor::MetricEntry;
    use rtactor as rt;

    #[derive(SyncNotifier)]
    pub enum Notification {
        MetricUpdate {
            /// Instant of the first snapshot request.
            timestamp: rt::Instant,
            names_metric_lists: Vec<(String, Vec<MetricEntry>)>,
        },
    }

    define_sim_sync_accessor!(SimSyncAccessor, SyncNotifier);
    define_sync_accessor!(SyncAccessor, SyncNotifier);
}

// ---------- public types ---------------

pub struct ProfilingAggregator {
    actor_entry_dict: BTreeMap<String, ActorEntry>,
    observer_set: BTreeSet<rt::Addr>,
    state: State,
}

// --------------- public implementations -----------

impl ProfilingAggregator {
    pub fn new() -> Self {
        Self {
            actor_entry_dict: BTreeMap::new(),
            observer_set: BTreeSet::new(),
            state: State::Idle,
        }
    }
}

impl Default for ProfilingAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl rt::Behavior for ProfilingAggregator {
    fn process_message(&mut self, context: &mut rt::ProcessContext, msg: &rt::Message) {
        match msg {
            rt::Message::Request(request) => self.process_request(context, request),
            rt::Message::Response(response) => self.process_response(context, response),
            rt::Message::Notification(notification) => {
                self.process_notification(context, notification)
            }
        }
    }
}

// --------------- private types -----------

const INITIAL_METRIC_LIST_CAPACITY: usize = 20;

enum State {
    Idle,
    Processing {
        snapshot_list: Vec<String>,
        timestamp: rt::Instant, // first snapshot request
        requested_index: usize,
        request_id: rt::RequestId,
    },
}

struct ActorEntry {
    addr: rt::Addr,
    metric_list: Cell<Option<Vec<MetricEntry>>>,
}

// --------------- private implementations -----------

impl ActorEntry {
    pub fn new(addr: &rt::Addr) -> Self {
        Self {
            addr: addr.clone(),
            metric_list: Cell::new(Some(Vec::with_capacity(INITIAL_METRIC_LIST_CAPACITY))),
        }
    }
}

impl ProfilingAggregator {
    fn convert_selection_to_names(&self, selection: &ActorSelection) -> Vec<String> {
        match selection {
            ActorSelection::All => self.actor_entry_dict.keys().cloned().collect(),
            ActorSelection::SelectedNames(names) => names.clone(),
            ActorSelection::SelectedAddresses(addresses) => {
                let mut names = Vec::with_capacity(addresses.len());
                for it in self.actor_entry_dict.iter() {
                    if addresses.contains(&it.1.addr) {
                        names.push(it.0.clone());
                    }
                }
                names
            }
        }
    }

    fn process_request(&mut self, context: &mut rt::ProcessContext, request: &rt::Request) {
        if let Some(data) = request.data.downcast_ref() {
            match data {
                Request::RegisterObserver { addr } => {
                    self.observer_set.insert(addr.clone());
                    context.send_response(request, Response::RegisterObserver());
                }
                Request::UnregisterObserver { addr } => {
                    self.observer_set.remove(addr);
                    context.send_response(request, Response::UnregisterObserver());
                }
                Request::RegisterProfiledActors { names_addresses } => {
                    for (name, addr) in names_addresses {
                        self.actor_entry_dict
                            .insert(name.clone(), ActorEntry::new(addr));
                    }
                    context.send_response(request, Response::RegisterProfiledActors());
                }
                Request::UnregisterProfiledActors { selection } => {
                    match selection {
                        ActorSelection::All => self.actor_entry_dict.clear(),
                        ActorSelection::SelectedNames(names) => {
                            for name in names {
                                self.actor_entry_dict.remove(name);
                            }
                        }
                        ActorSelection::SelectedAddresses(addresses) => {
                            self.actor_entry_dict
                                .retain(|_, entry| !addresses.contains(&entry.addr));
                        }
                    };
                    context.send_response(request, Response::UnregisterProfiledActors());
                }
            }
        }
    }

    fn process_next_snapshot_and_notify(&mut self, context: &mut rt::ProcessContext) {
        let mut list_and_timestamp_to_notify = None;

        match &mut self.state {
            State::Idle => {}
            State::Processing {
                snapshot_list,
                timestamp,
                requested_index,
                request_id,
            } => {
                let mut request_sent = false;

                for name in snapshot_list.iter().skip(*requested_index) {
                    if let Some(entry) = self.actor_entry_dict.get(name) {
                        let list = match entry.metric_list.take() {
                            Some(mut list) => {
                                list.clear();
                                list
                            }
                            None => Vec::with_capacity(INITIAL_METRIC_LIST_CAPACITY),
                        };

                        *request_id = context.send_request(
                            &entry.addr,
                            profiled_actor::Request::GetMetrics {
                                metric_list: Cell::new(Some(list)),
                            },
                        );
                        request_sent = true;
                        break;
                    } else {
                        *requested_index += 1;
                    }
                }

                if !request_sent {
                    list_and_timestamp_to_notify = Some((snapshot_list.clone(), *timestamp));
                }
            }
        }

        if let Some((list_to_notify, timestamp)) = list_and_timestamp_to_notify {
            self.notify_observers(context, &list_to_notify, &timestamp);
            self.state = State::Idle;
        }
    }

    fn process_response(&mut self, context: &mut rt::ProcessContext, response: &rt::Response) {
        if let State::Processing {
            snapshot_list,
            requested_index,
            request_id,
            ..
        } = &mut self.state
        {
            match &response.result {
                Ok(data) => {
                    if let Some(data) = data.downcast_ref() {
                        match data {
                            profiled_actor::Response::GetMetrics(entry_list) => {
                                if response.id_eq(*request_id) {
                                    if let Some(entry) = self
                                        .actor_entry_dict
                                        .get_mut(&*snapshot_list[*requested_index])
                                    {
                                        entry.metric_list.swap(entry_list)
                                    }
                                    *requested_index += 1;

                                    self.process_next_snapshot_and_notify(context);
                                }
                            }
                        }
                    }
                }
                Err(error_status) => {
                    if let Some(data) = error_status.request_data.downcast_ref() {
                        match data {
                            profiled_actor::Response::GetMetrics(_) => {
                                if response.id_eq(*request_id) {
                                    self.process_next_snapshot_and_notify(context);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn notify_observers(
        &mut self,
        context: &mut rt::ProcessContext,
        names: &Vec<String>,
        timestamp: &rt::Instant,
    ) {
        let mut names_metric_lists = Vec::with_capacity(names.len());

        for name in names {
            if let Some(entry) = self.actor_entry_dict.get_mut(name) {
                let list = match entry.metric_list.get_mut() {
                    Some(list) => list.clone(),
                    None => Vec::new(),
                };
                names_metric_lists.push((name.clone(), list));
            }
        }

        for observer in &self.observer_set {
            let _ = context.send_notification(
                observer,
                observer::Notification::MetricUpdate {
                    names_metric_lists: names_metric_lists.clone(),
                    timestamp: *timestamp,
                },
            );
        }
    }

    fn process_notification(
        &mut self,
        context: &mut rt::ProcessContext,
        notification: &rt::Notification,
    ) {
        if let Some(data) = notification.data.downcast_ref() {
            match data {
                Notification::SnapshotMetrics { selection } => {
                    let new_snapshot_list = self.convert_selection_to_names(selection);
                    match &mut self.state {
                        State::Idle => {
                            self.state = State::Processing {
                                snapshot_list: new_snapshot_list,
                                timestamp: context.now(),
                                requested_index: 0,
                                request_id: 0,
                            };

                            self.process_next_snapshot_and_notify(context);
                        }
                        State::Processing { snapshot_list, .. } => {
                            // add requested snapshot not already done to the end of the list.
                            for name in new_snapshot_list {
                                if !snapshot_list.contains(&name) {
                                    snapshot_list.push(name);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
