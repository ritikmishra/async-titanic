use async_executor::Executor;
use futures::pin_mut;
use mio::net::TcpStream;
use reactor::Reactor;

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
    let reactor = Reactor::new();

    let sock1 = TcpStream::connect("54.208.105.16:80".parse().unwrap()).unwrap();
    let sock2 = TcpStream::connect("54.208.105.16:80".parse().unwrap()).unwrap();

    let mut sock1 = reactor.register_new_socket(sock1);
    let mut sock2 = reactor.register_new_socket(sock2);

    let fut1 = async {
        println!("sending data (fut 1)");
        sock1.write(REQUEST_CONTENT_1S.as_bytes()).await.unwrap();
        println!("sent data (fut 1)");

        println!("recieving data (fut 1)");
        let data = sock1.read().await.unwrap();
        print!("{}", String::from_utf8_lossy(&data));

        println!("all done! (fut 1)");
    };

    let fut2 = async {
        println!("sending data (fut 2)");
        sock2.write(REQUEST_CONTENT_4S.as_bytes()).await.unwrap();
        println!("sent data (fut 2)");

        println!("recieving data (fut 2)");
        let data = sock2.read().await.unwrap();
        print!("{}", String::from_utf8_lossy(&data));

        println!("all done! (fut 2)");
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
        io::{self, ErrorKind, Read, Write},
        pin::Pin,
        task::{Context, Poll},
    };

    use futures::Future;

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

    impl<'a> Future for TcpStreamReadFut<'a> {
        type Output = io::Result<Vec<u8>>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let sockets = self.reactor.sockets.borrow();
            let mut borrow_mut = sockets[self.socket_idx].borrow_mut();
            let mut socket = borrow_mut.as_mut().unwrap();

            let mut buf = Vec::new();
            let read_result = socket.stream.read_to_end(&mut buf);

            let ret = {
                match read_result {
                    Ok(_) => Poll::Ready(Ok(buf)),
                    Err(e) => match e.kind() {
                        ErrorKind::WouldBlock if buf.len() > 0 => Poll::Ready(Ok(buf)),
                        ErrorKind::NotConnected | ErrorKind::WouldBlock => Poll::Pending,
                        _ => Poll::Ready(Err(e)),
                    },
                }
            };

            if matches!(ret, Poll::Pending) {
                socket.waker = Some(cx.waker().clone());
            }

            ret
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
            let sockets = self.reactor.sockets.borrow();
            let mut borrow_mut = sockets[self.socket_idx].borrow_mut();
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
    use std::io;
    use std::task::Waker;
    use std::time::Duration;

    use mio::net::TcpStream;
    use mio::{Events, Interest, Poll, Token};

    use crate::socket_stream::{TcpStreamReadFut, TcpStreamWriteFut};

    pub struct TcpStreamWrapper {
        pub stream: TcpStream,
        pub waker: Option<Waker>,
    }

    pub struct AsyncSocket<'a> {
        reactor: &'a Reactor,
        socket_idx: usize,
    }

    impl AsyncSocket<'_> {
        pub async fn read(&mut self) -> io::Result<Vec<u8>> {
            TcpStreamReadFut::new(self.reactor, self.socket_idx).await
        }

        pub async fn write(&mut self, to_write: &[u8]) -> io::Result<()> {
            TcpStreamWriteFut::new(self.reactor, self.socket_idx, to_write).await;
            Ok(())
        }
    }

    pub struct Reactor {
        poll: RefCell<Poll>,
        pub sockets: RefCell<Vec<RefCell<Option<TcpStreamWrapper>>>>,
    }
    impl Reactor {
        pub fn new() -> Self {
            Self {
                poll: Poll::new().unwrap().into(),
                sockets: Vec::new().into(),
            }
        }

        pub fn register_new_socket(&self, mut socket: TcpStream) -> AsyncSocket {
            let mut sockets = self.sockets.borrow_mut();
            let poll = self.poll.borrow_mut();
            let new_socket_idx = sockets.len();
            poll.registry()
                .register(
                    &mut socket,
                    Token(new_socket_idx),
                    Interest::READABLE | Interest::WRITABLE,
                )
                .unwrap();
            sockets.push(RefCell::new(Some(TcpStreamWrapper {
                stream: socket,
                waker: None,
            })));
            AsyncSocket {
                reactor: self,
                socket_idx: new_socket_idx,
            }
        }

        pub fn use_socket(&self, socket_idx: usize) -> AsyncSocket {
            AsyncSocket {
                reactor: self,
                socket_idx,
            }
        }

        pub fn tick(&self) {
            let mut events = Events::with_capacity(128);
            let sockets = self.sockets.borrow();
            let mut poll = self.poll.borrow_mut();
            poll.poll(&mut events, Some(Duration::from_millis(1)))
                .unwrap();
            for event in &events {
                // find the socket we got an event for, and wake up that future
                let Token(socket_idx) = event.token();
                println!("received event for {} ({:?})", socket_idx, event);
                if let Some(waker) = sockets[socket_idx]
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
            let sockets = self.sockets.borrow();
            let mut borrow = sockets[socket_idx].borrow_mut();
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
