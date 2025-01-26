//! ReactiveMocker is a helper that allows to mock more easily rtactor::Behavior.
//!
//! For complex use cases MockBehavior is fairly limited:
//!
//! * the expectations must be provided before transferring the reactive to the dispatcher
//! * checkpoint() can not be used because of this limitation
//! * addresses of other mock are not already created because of this limitation
//! * accessing data from the test in the Behavior is complex, and involve Mutex or Request
//! * a Message type downcast is needed every time and is not very convenient
//!
//! This helper fixes these problems. Although unit tests should be written single-threaded,
//! this helper is thread-safe.

use std::{
    cell::RefCell,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

use crate::{simulation::SimulationDispatcher, Addr, Behavior, Message, ProcessContext};

/// An helper that allows to access a custom T data inside mock methods.
pub struct ReactiveMocker<T: 'static> {
    addr: Addr,
    mock: Arc<Mutex<MockReactive<T>>>,
    data: Arc<Mutex<T>>,
}

/// A mockall reactive mock that split process_message() and add data as argument.
#[mockall::automock]
pub trait Reactive<T> {
    fn process_request<'a, 'b>(
        &mut self,
        mock_data: &'a mut T,
        context: &'a mut ProcessContext<'b>,
        request: &crate::Request,
    );
    fn process_response<'a, 'b>(
        &mut self,
        mock_data: &'a mut T,
        context: &'a mut ProcessContext<'b>,
        response: &crate::Response,
    );
    fn process_notification<'a, 'b>(
        &mut self,
        mock_data: &'a mut T,
        context: &'a mut ProcessContext<'b>,
        request: &crate::Notification,
    );
}

impl<T> ReactiveMocker<T> {
    /// Create a mocker and add it to a dispatcher with the initial T data.
    pub fn new(dispatcher: &RefCell<SimulationDispatcher>, data: T) -> Self {
        let mut me = Self {
            addr: Addr::INVALID,
            mock: Arc::new(Mutex::new(MockReactive::new())),
            data: Arc::new(Mutex::new(data)),
        };
        me.register(dispatcher);
        me
    }

    /// Allows access to the T data.
    pub fn data(&self) -> MutexGuard<'_, T> {
        self.data.lock().unwrap()
    }

    pub fn data_to_weak(&self) -> Weak<Mutex<T>> {
        Arc::downgrade(&self.data)
    }

    /// Allows access to the mockall mock.
    pub fn mock(&self) -> MutexGuard<'_, MockReactive<T>> {
        self.mock.lock().unwrap()
    }

    /// Get a copy of the reactive address.
    pub fn addr(&self) -> Addr {
        self.addr.clone()
    }

    fn register(&mut self, dispatcher: &RefCell<SimulationDispatcher>) {
        self.addr = dispatcher
            .borrow_mut()
            .register_reactive(Box::new(ReactiveMocker {
                addr: Addr::INVALID, // address is not useful inside the rtactor because context provide it
                mock: self.mock.clone(),
                data: self.data.clone(),
            }));
    }
}

impl<T> Behavior for ReactiveMocker<T> {
    fn process_message<'a, 'b>(&mut self, context: &'a mut ProcessContext<'b>, msg: &Message) {
        let data = &mut *self.data.lock().unwrap();

        let mut mock = self.mock.lock().unwrap();

        match msg {
            Message::Request(request) => mock.process_request(data, context, request),
            Message::Response(response) => mock.process_response(data, context, response),
            Message::Notification(notification) => {
                mock.process_notification(data, context, notification)
            }
        }
    }
}
