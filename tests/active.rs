use assert2::let_assert;
use rtactor::{ActiveActor, Addr, Error, Message};
use std::cell::RefCell;
use std::time::Duration;
use std::vec::Vec;

#[test]
fn test_addr() {
    let mut addr = Addr::new();
    assert!(!addr.is_valid());

    let actor = ActiveActor::new(1);
    addr = actor.addr();
    assert!(addr.is_valid());
}

#[test]
/// Check if it's possible to send a buffer and extract it from a immutable Message reference.
/// This is important because this way it's possible not to do a memory copy of large buffers passed.
fn send_buffer_without_copy() {
    let mut receiver = ActiveActor::new(1);

    enum Notification {
        Buffer(RefCell<Option<Vec<u8>>>),
    }

    rtactor::send_notification(
        &receiver.addr(),
        Notification::Buffer(RefCell::new(Some(vec![0, 3, 5, 7]))),
    )
    .unwrap();

    let msg = &receiver.wait_message().unwrap();

    match msg {
        Message::Notification(notif) => {
            let notif = notif.data.downcast_ref::<Notification>().unwrap();

            match notif {
                Notification::Buffer(cell) => {
                    let v = cell.borrow_mut().take().unwrap();
                    assert_eq!(v, vec!(0, 3, 5, 7))
                }
            }
        }
        _ => panic!(),
    }
}

#[test]
/// Send a request from an active actor and use ActiveActor::responds() to respond.
fn test_responds() {
    let mut requester = ActiveActor::new(1);

    let_assert!(
        Err(Error::AddrUnreachable) =
            requester.request_for::<_, u32>(&Addr::INVALID, 0x1234, Duration::from_secs(5))
    );
    let requested_full_queue = ActiveActor::new(1);
    rtactor::send_notification(&requested_full_queue.addr(), 23).unwrap();

    let_assert!(
        Err(Error::QueueFull) =
            requester.request_for::<_, u32>(&requested_full_queue.addr(), 0x1234, Duration::ZERO)
    );

    let requester_addr = requester.addr();
    let mut requested = ActiveActor::new(1);
    let requested_addr = requested.addr();

    let join_handle = std::thread::spawn(move || {
        let_assert!(Ok(Message::Request(request)) = requested.wait_message());
        let_assert!(Some(val_ref) = request.data.downcast_ref::<i32>());
        let val = *val_ref;
        assert_eq!(val, -123);
        assert_eq!(request.src, requester_addr);
        let_assert!(Ok(()) = requested.responds::<i32>(request, -val));
    });

    let_assert!(
        Ok(ret_val) =
            requester.request_for::<_, i32>(&requested_addr, -123i32, Duration::from_secs(5))
    );

    assert_eq!(ret_val, 123);
    join_handle.join().unwrap();
}
