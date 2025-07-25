use super::actor::{self, Message, RequestId};
use crate::Request;
use std::boxed::Box;
use std::collections::LinkedList;
use std::time::Duration;

////////////////////////////// public types /////////////////////////////////////

/// Actor that has it own message queue and manage actively how to wait on it.
pub struct AsyncMailbox {
    id: actor::ActorId,
    rx: async_channel::Receiver<Message>,
    tx: async_channel::Sender<Message>,
    last_request_id: RequestId,
    /// list to store messages popped from rx but not consumed because of filtered receive.
    message_list: LinkedList<Message>,
}

/// Super trait used by AsyncRequester and AsyncNotifier to send requests and notifications
///
/// These trait are generated by `#[derive(AsyncRequester)]` and `#[derive(AsyncNotifier)]`
/// and provide an async interface to an actor.
#[allow(async_fn_in_trait)]
pub trait AsyncAccessor {
    fn send_notification<T>(&mut self, data: T) -> Result<(), crate::Error>
    where
        T: 'static + Send;
    async fn request_for<TRequest, TResponse>(
        &mut self,
        request_data: TRequest,
        timeout: Duration,
    ) -> Result<TResponse, crate::Error>
    where
        TRequest: 'static + Send + Sized,
        TResponse: 'static + Send + Sized;
}

////////////////////////////// internal types /////////////////////////////////////

/// Address implementation to an `AsyncMailbox`
#[derive(Debug, Clone)]
pub(crate) struct AsyncAddr {
    id: actor::ActorId,
    tx: async_channel::Sender<Message>,
}

////////////////////////////// public macros /////////////////////////////////////

/// Create a struct that allows a blocking access to a address and implements one or more AsyncRequester or AsyncNotifier
///
/// Example:
/// ```rs
/// define_async_accessor(MyAsyncAccessorStructName, FirstAsyncRequesterName, SecondAsyncRequesterName, ...);
/// ```
/// The struct implements `MyAsyncAccessorStructName`. It will have a function `new(&Addr) -> MyAsyncAccessorStructName`
/// and implement all traits following the structName without changes of they default implementation.
#[macro_export]
macro_rules! define_async_accessor{
    ($async_accessor_name:ident, $($async_trait:ty),+)
    =>
    {
        pub struct $async_accessor_name {
            async_actor: ::rtactor::AsyncMailbox,
            target_addr: ::rtactor::Addr,
        }

        impl $async_accessor_name {
            pub fn new(target_addr: &::rtactor::Addr) -> $async_accessor_name {
                $async_accessor_name {
                    async_actor: ::rtactor::AsyncMailbox::new(1),
                    target_addr: target_addr.clone(),
                }
            }

            pub fn target_addr(&self)-> &::rtactor::Addr {&self.target_addr}
        }

        impl ::rtactor::AsyncAccessor for $async_accessor_name {
            fn send_notification<T>(&mut self, data: T) -> Result<(), ::rtactor::Error>
            where
            T: 'static + Send,
            {
                let addr = self.target_addr.clone();
                self.async_actor.send_notification(&addr, data)
            }
            async fn request_for<TRequest, TResponse>(
                &mut self,
                request_data: TRequest,
                timeout: std::time::Duration,
            ) -> Result<TResponse, ::rtactor::Error>
            where
                TRequest: 'static + Send + Sized,
                TResponse: 'static + Send + Sized
            {
                let addr = self.target_addr.clone();
                self.async_actor.request_for(&addr, request_data, timeout).await
            }

        }

        $(
            impl $async_trait for $async_accessor_name {}
        )*
    }
}

////////////////////////////// public impl's /////////////////////////////////////
impl AsyncMailbox {
    /// Create an async actor with a given maximum queue.
    pub fn new(queue_size: usize) -> AsyncMailbox {
        let (tx, rx) = async_channel::bounded(queue_size);
        AsyncMailbox {
            id: actor::generate_actor_id(),
            rx,
            tx,
            last_request_id: 0,
            message_list: LinkedList::new(),
        }
    }

    /// Get the address of the actor.
    pub fn addr(&self) -> actor::Addr {
        actor::Addr {
            kind: actor::AddrKind::Async(AsyncAddr {
                id: self.id,
                tx: self.tx.clone(),
            }),
        }
    }

    /// Try to get a single message.
    ///
    /// In case of error, can be `Error::NoMessage` or an error during the
    /// pop of the queue.
    pub fn try_get_message(&mut self) -> Result<Message, actor::Error> {
        if let Some(message) = self.message_list.pop_back() {
            return Result::Ok(message);
        }

        match self.rx.try_recv() {
            Ok(message) => Ok(message),
            Err(async_channel::TryRecvError::Empty) => Err(actor::Error::NoMessage),
            Err(async_channel::TryRecvError::Closed) => Err(actor::Error::BrokenReceive),
        }
    }

    /// Wait Indefinitely for a message.
    pub async fn wait_message(&mut self) -> Result<Message, actor::Error> {
        self.wait_message_for(Duration::MAX).await
    }

    /// Wait for receiving a message
    /// This function will use different strategies depending on the feature flags:
    /// - If just `async-actor` is enabled, it will never timeout and wait indefinitely.
    /// - If `async-tokio` is enabled, it will use tokio's async runtime with a timeout.
    /// - If `async-smol` is enabled, it will use smol's async runtime with a timeout.
    /// - If all features are enable it will use the tokio runtime.
    #[allow(unreachable_code)]
    #[allow(unused_variables)]
    async fn wait_for_rx_message(&mut self, timeout: Duration) -> Result<Message, actor::Error> {
        #[cfg(feature = "async-tokio")]
        {
            let result = match tokio::time::timeout(timeout, self.rx.recv()).await {
                Ok(Ok(message)) => Ok(message),
                Ok(Err(_)) => Err(actor::Error::BrokenReceive),
                Err(_) => Err(actor::Error::Timeout),
            };
            return result;
        }

        // This is only reachable if the `async-tokio` feature is not enabled.
        #[cfg(feature = "async-smol")]
        {
            use smol::Timer;

            let recv_future = async {
                match self.rx.recv().await {
                    Ok(message) => Ok(Ok(message)),
                    Err(_) => Ok(Err(actor::Error::BrokenReceive)),
                }
            };
            let timeout_future = async {
                Timer::after(timeout).await;
                Err(actor::Error::Timeout)
            };

            let result = match smol::future::race(recv_future, timeout_future).await {
                Ok(result) => result,
                Err(timeout_err) => Err(timeout_err),
            };
            return result;
        }

        // This is only reachable if neither the `async-tokio` nor `async-smol` features are enabled.
        // In this case, we will wait indefinitely.
        let result = self.rx.recv().await;
        match result {
            Ok(message) => Ok(message),
            Err(async_channel::RecvError) => Err(actor::Error::BrokenReceive),
        }
    }

    /// Wait for a message for a given amount of duration.
    pub async fn wait_message_for(&mut self, timeout: Duration) -> Result<Message, actor::Error> {
        // look in linked list
        if let Some(message) = self.message_list.pop_back() {
            return Result::Ok(message);
        }

        // wait on the queue with timeout
        self.wait_for_rx_message(timeout).await
    }

    /// Send notification.
    ///
    /// This method is equivalent to the function rtactor::send_notification()
    /// but is preferred because it allows possible future thread local memory allocation.
    pub fn send_notification<T>(&mut self, dst: &actor::Addr, data: T) -> Result<(), crate::Error>
    where
        T: 'static + Send,
    {
        dst.receive_notification(data)
    }

    /// Send back an ok response to this request.
    pub fn responds<T>(&mut self, request: Request, data: T) -> Result<(), crate::Error>
    where
        T: 'static + Send,
    {
        request.src.receive_ok_response(request.id, data)
    }

    /// Send a request and wait for the response for a given duration.
    pub async fn request_for<TRequest, TResponse>(
        &mut self,
        dst: &actor::Addr,
        request_data: TRequest,
        timeout: Duration,
    ) -> Result<TResponse, actor::Error>
    where
        TRequest: 'static + Send + Sized,
        TResponse: 'static + Send + Sized,
    {
        let request_id = self.generate_request_id();
        dst.receive_request(&self.addr(), request_id, request_data);

        loop {
            let result = self.wait_for_rx_message(timeout).await;

            match result {
                Ok(actor::Message::Response(response)) => match response.result {
                    Ok(data) => match data.downcast::<TResponse>() {
                        Ok(out) => {
                            if response.request_id == request_id {
                                return Ok(*out);
                            } else {
                                continue;
                            }
                        }
                        Err(result) => {
                            self.message_list.push_back(actor::Message::Response(
                                actor::Response {
                                    request_id: response.request_id,
                                    result: Ok(result),
                                },
                            ));
                            continue;
                        }
                    },
                    Err(err) => {
                        return Err(err.error);
                    }
                },
                Ok(msg) => {
                    self.message_list.push_back(msg);
                    continue;
                }
                Err(_) => {
                    return Err(actor::Error::BrokenReceive);
                }
            }
        }
    }

    pub(crate) fn generate_request_id(&mut self) -> RequestId {
        self.last_request_id = self.last_request_id.wrapping_add(1);
        self.last_request_id
    }
}

impl AsyncAddr {
    pub(crate) fn actor_id(&self) -> actor::ActorId {
        self.id
    }

    pub fn receive_notification<T>(&self, data: T) -> Result<(), actor::Error>
    where
        T: 'static + Send,
    {
        let datagram = Message::Notification(actor::Notification {
            data: Box::new(data),
        });

        match self.tx.try_send(datagram) {
            Ok(_) => Result::Ok(()),
            Err(err) => Result::Err(match err {
                async_channel::TrySendError::Full(_) => actor::Error::QueueFull,
                async_channel::TrySendError::Closed(..) => actor::Error::AddrUnreachable,
            }),
        }
    }

    pub fn receive_request<T>(&self, src: &actor::Addr, id: RequestId, data: T)
    where
        T: 'static + Send + Sized,
    {
        let message = Message::Request(actor::Request {
            src: src.clone(),
            id,
            data: Box::new(data),
        });

        if let Err(err) = self.tx.try_send(message) {
            let (actor_err, returned_message) = match err {
                async_channel::TrySendError::Full(a_message) => {
                    (actor::Error::QueueFull, a_message)
                }
                async_channel::TrySendError::Closed(a_message) => {
                    (actor::Error::AddrUnreachable, a_message)
                }
            };
            if let Message::Request(request) = returned_message {
                let _ = src.receive_err_response(
                    id,
                    actor::NonBoxedErrorStatus {
                        error: actor_err,
                        request_data: request.data,
                    },
                );
            }
        }
    }
    pub(super) fn receive_ok_response<T>(
        &self,
        request_id: RequestId,
        result: T,
    ) -> Result<(), actor::Error>
    where
        T: 'static + Send + Sized,
    {
        let response = actor::Response {
            request_id,
            result: Result::Ok(Box::new(result)),
        };

        match self.tx.try_send(actor::Message::Response(response)) {
            Ok(_) => Result::Ok(()),
            Err(err) => Result::Err(match err {
                async_channel::TrySendError::Full(_) => actor::Error::QueueFull,
                async_channel::TrySendError::Closed(..) => actor::Error::AddrUnreachable,
            }),
        }
    }

    pub(super) fn receive_err_response<T>(
        &self,
        request_id: RequestId,
        result: actor::NonBoxedErrorStatus<T>,
    ) -> Result<(), actor::Error>
    where
        T: 'static + Send + Sized,
    {
        let boxed_result = actor::ErrorStatus {
            error: result.error,
            request_data: Box::new(result.request_data),
        };

        let response = actor::Response {
            request_id,
            result: Result::Err(boxed_result),
        };

        match self.tx.try_send(actor::Message::Response(response)) {
            Ok(_) => Result::Ok(()),
            Err(err) => Result::Err(match err {
                async_channel::TrySendError::Full(_) => actor::Error::QueueFull,
                async_channel::TrySendError::Closed(..) => actor::Error::AddrUnreachable,
            }),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn generate_request_id() {
        let mut actor = AsyncMailbox::new(1);

        assert_eq!(actor.generate_request_id(), 1);

        assert_eq!(actor.generate_request_id(), 2);

        actor.last_request_id = RequestId::MAX - 1;
        assert_eq!(actor.generate_request_id(), RequestId::MAX);
        assert_eq!(actor.generate_request_id(), 0);
        assert_eq!(actor.generate_request_id(), 1);
    }
}
