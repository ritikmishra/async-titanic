use async_executor::Executor;
use futures::{pin_mut, StreamExt};
use mio::net::TcpStream;
use reactor::Reactor;

use crate::socket_stream::{TcpStreamReadFut, TcpStreamWriteFut};

const REQUEST_CONTENT_1S: &'static str = "GET /delay/1 HTTP/1.1
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
Accept-Encoding: identity
Accept-Language: en-US,en;q=0.9
Cache-Control: max-age=0
Connection: close
Host: httpbin.org
Upgrade-Insecure-Requests: 1
User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/102.0.0.0 Safari/537.36

";

const REQUEST_CONTENT_4S: &'static str = "GET /delay/4 HTTP/1.1
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
Accept-Encoding: identity
Accept-Language: en-US,en;q=0.9
Cache-Control: max-age=0
Connection: close
Host: httpbin.org
Upgrade-Insecure-Requests: 1
User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/102.0.0.0 Safari/537.36

";

fn main() {
    let mut executor = Executor::new();
    let mut reactor = Reactor::new();

    let sock1 = TcpStream::connect("54.208.105.16:80".parse().unwrap()).unwrap();
    let sock_id1 = reactor.register_new_socket(sock1);

    let sock2 = TcpStream::connect("54.208.105.16:80".parse().unwrap()).unwrap();
    let sock_id2 = reactor.register_new_socket(sock2);

    let fut1 = async {
        // hi
        println!("sending data");
        TcpStreamWriteFut::new(&reactor, sock_id1, REQUEST_CONTENT_1S.as_bytes()).await;
        println!("sent data");

        println!("recieving data");
        while let Some(data) = TcpStreamReadFut::new(&reactor, sock_id1).next().await {
            print!("{}", String::from_utf8_lossy(&data));
        }

        println!("all done!");
    };

    let fut2 = async {
        // hi
        println!("sending data");
        TcpStreamWriteFut::new(&reactor, sock_id2, REQUEST_CONTENT_4S.as_bytes()).await;
        println!("sent data");

        println!("recieving data");
        while let Some(data) = TcpStreamReadFut::new(&reactor, sock_id2).next().await {
            print!("{}", String::from_utf8_lossy(&data));
        }

        println!("all done!");
    };

    pin_mut!(fut1, fut2);

    executor.spawn(fut1);
    executor.spawn(fut2);
    loop {
        executor.step_once();
        reactor.tick();
        if executor.all_tasks_done() {
            break;
        }
    }
}

mod socket_stream {
    use std::{
        io::{ErrorKind, Read, Write},
        pin::Pin,
        task::{Context, Poll},
    };

    use futures::{stream::Stream, Future};

    use crate::reactor;

    pub struct TcpStreamReadFut<'a> {
        reactor: &'a reactor::Reactor,
        socket_idx: usize,
    }

    impl<'a> TcpStreamReadFut<'a> {
        pub fn new(reactor: &'a reactor::Reactor, socket_idx: usize) -> Self {
            Self {
                reactor,
                socket_idx,
            }
        }
    }

    impl<'a> Stream for TcpStreamReadFut<'a> {
        type Item = Vec<u8>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            // unwrap because they shouldn't close our socket
            let mut borrow_mut = self.reactor.sockets[self.socket_idx].borrow_mut();
            let mut socket = borrow_mut.as_mut().unwrap();
            let mut buf = Vec::new();
            let read_result = socket.stream.read_to_end(&mut buf);

            socket.waker = Some(cx.waker().clone());

            // poll socket
            match read_result {
                Ok(0) => {
                    socket.waker = None;
                    Poll::Ready(None)
                }
                // ready -> yield data
                Ok(_) => Poll::Ready(Some(buf)),
                Err(e) => match e.kind() {
                    // not ready -> place on queue
                    ErrorKind::WouldBlock => {
                        // remember to wake us when the future is ready
                        println!("hopefully someone wakes me up");

                        if buf.len() > 0 {
                            Poll::Ready(Some(buf))
                        } else {
                            Poll::Pending
                        }
                    }
                    ErrorKind::BrokenPipe
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::ConnectionRefused
                    | ErrorKind::ConnectionReset
                    | ErrorKind::NotConnected => {
                        socket.waker = None;
                        Poll::Ready(None)
                    }
                    _ => {
                        panic!("encountered unknown error: {:?}", e)
                    }
                },
            }
        }
    }

    pub struct TcpStreamWriteFut<'a, 'b> {
        reactor: &'a reactor::Reactor,
        socket_idx: usize,
        data: &'b [u8],
        bytes_written: usize,
    }

    impl<'a, 'b> TcpStreamWriteFut<'a, 'b> {
        pub fn new(reactor: &'a reactor::Reactor, socket_idx: usize, data: &'b [u8]) -> Self {
            Self {
                reactor,
                socket_idx,
                data,
                bytes_written: 0,
            }
        }
    }

    impl<'a> Future for TcpStreamWriteFut<'a, '_> {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            // unwrap because they shouldn't close our socket
            let mut borrow_mut = self.reactor.sockets[self.socket_idx].borrow_mut();
            let mut socket = borrow_mut.as_mut().unwrap();
            let read_result = socket.stream.write(&self.data[self.bytes_written..]);

            // poll socket
            match read_result {
                // ready -> yield data
                Ok(bytes_written) => {
                    println!(
                        "wrote {} more bytes ({} bytes so far, {} to write total)",
                        bytes_written,
                        self.bytes_written,
                        self.data.len()
                    );
                    self.bytes_written += bytes_written;
                    if self.bytes_written == self.data.len() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                }
                Err(e) => match e.kind() {
                    // not ready -> place on queue
                    ErrorKind::WouldBlock | ErrorKind::NotConnected => {
                        // remember to wake us when the future is ready
                        socket.waker = Some(cx.waker().clone());
                        Poll::Pending
                    }
                    // ErrorKind::BrokenPipe
                    // | ErrorKind::ConnectionAborted
                    // | ErrorKind::ConnectionRefused
                    // | ErrorKind::ConnectionReset
                    // | ErrorKind::NotConnected => Poll::Ready(()),
                    _ => {
                        panic!("encountered unknown error: {:?}", e)
                    }
                },
            }
        }
    }

    // impl<'a> Drop for TcpStreamFut<'a> {
    //     fn drop(&mut self) {
    //         // close the socket that we are associated to
    //         self.reactor.close_connection(self.socket_idx)
    //     }
    // }
}

mod async_executor {
    use std::{
        pin::Pin,
        sync::mpsc::{channel, Receiver, Sender},
        task::Context,
    };

    use futures::Future;

    use crate::safe_waker::{WakeByIdx, WakerTraitExt};

    type FutPointer<'tasks> = Pin<&'tasks mut dyn Future<Output = ()>>;

    pub struct Executor<'tasks> {
        all_tasks: Vec<Option<FutPointer<'tasks>>>,
        pollable_tasks: Receiver<usize>,
        task_sender: Sender<usize>,
    }

    impl<'tasks> Executor<'tasks> {
        pub fn new() -> Self {
            let (sender, receiver) = channel();
            Self {
                all_tasks: Vec::new(),
                pollable_tasks: receiver,
                task_sender: sender,
            }
        }

        pub fn spawn(&mut self, fut: FutPointer<'tasks>) {
            self.all_tasks.push(Some(fut));
            self.task_sender
                .send(self.all_tasks.len() - 1)
                .expect("task_sender should never fail while the executor is alive")
        }

        pub fn step_once(&mut self) {
            // true if we polled at least 1 future
            while let Ok(fut_idx) = self.pollable_tasks.try_recv() {
                if let Some(fut_pointer) = &mut self.all_tasks[fut_idx] {
                    // remember we polled at least one future
                    let waker = WakeByIdx {
                        idx: fut_idx,
                        sender: self.task_sender.clone(),
                    }
                    .to_waker();
                    let mut context = Context::from_waker(&waker);
                    match fut_pointer.as_mut().poll(&mut context) {
                        std::task::Poll::Ready(()) => self.all_tasks[fut_idx] = None,
                        std::task::Poll::Pending => {}
                    }
                }
            }
        }

        pub fn all_tasks_done(&self) -> bool {
            self.all_tasks.iter().all(Option::is_none)
        }
    }
}

mod reactor {
    use std::cell::RefCell;
    use std::task::Waker;
    use std::time::Duration;

    use mio::net::TcpStream;
    use mio::{Events, Interest, Poll, Token};

    pub struct TcpStreamWrapper {
        pub stream: TcpStream,
        pub waker: Option<Waker>,
    }

    pub struct Reactor {
        poll: RefCell<Poll>,
        pub sockets: Vec<RefCell<Option<TcpStreamWrapper>>>,
    }
    impl Reactor {
        pub fn new() -> Self {
            Self {
                poll: Poll::new().unwrap().into(),
                sockets: Vec::new(),
            }
        }

        pub fn register_new_socket(&mut self, mut socket: TcpStream) -> usize {
            let new_socket_idx = self.sockets.len();
            let poll = self.poll.borrow_mut();
            poll.registry()
                .register(
                    &mut socket,
                    Token(new_socket_idx),
                    Interest::READABLE | Interest::WRITABLE,
                )
                .unwrap();
            self.sockets.push(RefCell::new(Some(TcpStreamWrapper {
                stream: socket,
                waker: None,
            })));
            new_socket_idx
        }
        pub fn tick(&self) {
            let mut events = Events::with_capacity(128);
            let mut poll = self.poll.borrow_mut();
            poll.poll(&mut events, Some(Duration::from_millis(1)))
                .unwrap();
            for event in &events {
                // find the socket we got an event for, and wake up that future
                let Token(socket_idx) = event.token();
                println!("received event for {} ({:?})", socket_idx, event);
                if let Some(waker) = self.sockets[socket_idx]
                    .borrow_mut()
                    .as_mut()
                    .expect("got event on a socket that we don't own??")
                    .waker
                    .take()
                {
                    println!("waking");
                    waker.wake()
                }
            }
        }

        pub fn close_connection(&self, socket_idx: usize) {
            let mut borrow = self.sockets[socket_idx].borrow_mut();
            if let Some(socket) = &mut *borrow {
                socket
                    .stream
                    .shutdown(std::net::Shutdown::Both)
                    .expect("why can't we shut down the socket?");
            }
            *borrow = None;
        }
    }
}

mod safe_waker {
    use std::{
        sync::mpsc::Sender,
        task::{RawWaker, RawWakerVTable, Waker},
    };

    mod waker_trait {
        use std::task::RawWakerVTable;

        pub trait WakerTrait: Clone {
            const VTABLE_PTR: &'static RawWakerVTable;
            fn wake(self) {
                self.wake_by_ref()
            }
            fn wake_by_ref(&self);

            fn to_pointer(self) -> *const () {
                Box::into_raw(Box::new(self)) as *const _ as *const ()
            }
        }
    }

    pub trait WakerTraitExt: waker_trait::WakerTrait {
        fn to_waker(self) -> Waker {
            unsafe { Waker::from_raw(RawWaker::new(self.to_pointer(), Self::VTABLE_PTR)) }
        }
    }
    impl<T: waker_trait::WakerTrait> WakerTraitExt for T {}

    #[macro_export]
    macro_rules! make_waker_vtable {
        ($vtable_name:ident, $waker_ty:ty) => {
            use $crate::safe_waker::waker_trait::WakerTrait;

            #[allow(unused)]
            const fn assert_implements_waker_trait<
                T: $crate::safe_waker::waker_trait::WakerTrait,
            >() {
            }
            const _: () = assert_implements_waker_trait::<$waker_ty>();

            const $vtable_name: RawWakerVTable = RawWakerVTable::new(
                |self_ptr| {
                    let self_ref = unsafe { &*(self_ptr as *const $waker_ty) };
                    let new = Box::new(self_ref.clone());
                    let pointer = Box::into_raw(new) as *const _;
                    RawWaker::new(pointer, &$vtable_name)
                },
                |self_ptr| {
                    let self_box =
                        unsafe { Box::from_raw(self_ptr as *const $waker_ty as *mut $waker_ty) };
                    self_box.wake()
                },
                |self_ref| {
                    let self_ref = unsafe { &*(self_ref as *const $waker_ty) };
                    self_ref.wake_by_ref()
                },
                |self_ptr| {
                    drop(unsafe { Box::from_raw(self_ptr as *const $waker_ty as *mut $waker_ty) })
                },
            );
        };
    }

    make_waker_vtable!(WAKE_BY_IDX_VTABLE, WakeByIdx);

    #[derive(Clone)]
    pub struct WakeByIdx {
        pub idx: usize,
        pub sender: Sender<usize>,
    }

    impl waker_trait::WakerTrait for WakeByIdx {
        const VTABLE_PTR: &'static RawWakerVTable = &WAKE_BY_IDX_VTABLE;

        fn wake_by_ref(&self) {
            self.sender.send(self.idx).unwrap();
        }
    }
}
