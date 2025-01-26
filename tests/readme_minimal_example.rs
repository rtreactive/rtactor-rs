use rtactor as rt;

struct Client {
    server_address: rt::Addr,
    dispatcher_address: rt::Addr,
    request_id: rt::RequestId,
}

impl Client {
    fn new(server_address: rt::Addr, dispatcher_address: rt::Addr) -> Client {
        Client {
            server_address,
            dispatcher_address,
            request_id: rt::RequestId::default(),
        }
    }
}

impl rt::Behavior for Client {
    fn process_message<'a, 'b>(
        &mut self,
        context: &'a mut rt::ProcessContext<'b>,
        message: &rt::Message,
    ) {
        match message {
            rt::Message::Notification(notification) => {
                if let Some(c) = notification.data.downcast_ref::<char>() {
                    println!("The client received a Notification with data='{}'.", c);
                    let str = format!("c={}", c);
                    self.request_id = context.send_request(&self.server_address, str);
                }
            }
            rt::Message::Response(response) => {
                if response.id_eq(self.request_id) {
                    if let Some(val) = response.result.as_ref().unwrap().downcast_ref::<i32>() {
                        println!("The client received a Response with result={}.", val);

                        context.send_request(
                            &self.dispatcher_address,
                            rt::dispatcher::Request::StopDispatcher {},
                        );
                        println!("The dispatcher will now stop.");
                        // The dispatcher will be stopped after that point
                        // so the response will never be processed.
                    }
                }
            }
            _ => panic!(),
        }
    }
}

struct Server();

impl rt::Behavior for Server {
    fn process_message<'a, 'b>(
        &mut self,
        context: &'a mut rt::ProcessContext<'b>,
        message: &rt::Message,
    ) {
        match message {
            rt::Message::Request(request) => {
                if let Some(str) = request.data.downcast_ref::<String>() {
                    println!("The server received a Request with data=\"{}\".", str);
                    context.send_response(request, 42i32);
                }
            }
            _ => panic!(),
        }
    }
}

#[test]
fn test() {
    use std::time::Duration;

    let disp_builder = rt::mpsc_dispatcher::Builder::new(10);
    let mut disp_accessor = disp_builder.to_accessor();

    let join_handle = std::thread::spawn(move || disp_builder.build().process());

    // The way an actor that is Send can be registered.
    let server_addr = disp_accessor
        .register_reactive_for(Server(), Duration::from_secs(10))
        .unwrap();

    // Registering inside the dispatcher thread is a bit more complex but allow to use actor
    // that are not Send.
    let client_addr = disp_accessor
        .execute_fn(
            move |disp| disp.register_reactive(Box::new(Client::new(server_addr, disp.addr()))),
            Duration::from_secs(10),
        )
        .unwrap();

    rt::send_notification(&client_addr, '_').unwrap();

    join_handle.join().unwrap();
}
