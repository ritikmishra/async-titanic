pub mod static_fut_storage;

pub mod socket_stream {
    use core::{
        pin::Pin,
        task::{Context, Poll},
    };
    use std::io::{self, ErrorKind, Read, Write};

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
        type Output = io::Result<()>;

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
                    self.bytes_written += bytes_written;
                    if self.bytes_written == self.data.len() {
                        Poll::Ready(Ok(()))
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
                    _ => Poll::Ready(Err(e)),
                },
            }
        }
    }
}

pub mod async_executor {
    use core::pin::Pin;
    use std::sync::mpsc::{channel, Receiver};

    use futures::Future;

    use crate::no_heap_waker;

    type FutPointer<'tasks> = Pin<&'tasks mut dyn Future<Output = ()>>;

    pub struct Executor<'tasks, const NUM_TASKS: usize> {
        all_tasks: [Option<(FutPointer<'tasks>, no_heap_waker::WakerData)>; NUM_TASKS],
        pollable_tasks: Receiver<usize>,
        // task_sender: Sender<usize>,
    }

    impl<'tasks, const NUM_TASKS: usize> Executor<'tasks, NUM_TASKS> {
        pub fn new_with_tasks(tasks: [FutPointer<'tasks>; NUM_TASKS]) -> Self {
            let (sender, receiver) = channel();
            // schedule all tasks to be executed initially
            for i in 0..NUM_TASKS {
                // SAFETY: this should never fail because the receiver is still in scope
                sender.send(i).unwrap();
            }

            // add some state around the futures
            let mut idx = 0;
            let all_tasks = tasks.map(|task| {
                let this_idx = idx;
                idx += 1;
                Some((
                    task,
                    no_heap_waker::WakerData {
                        idx: this_idx,
                        sender: sender.clone(),
                    },
                ))
            });

            Self {
                all_tasks,
                pollable_tasks: receiver,
                // task_sender: sender,
            }
        }

        pub fn step_once(&mut self) {
            while let Ok(fut_idx) = self.pollable_tasks.try_recv() {
                if let Some((fut_pointer, waker_data)) = &mut self.all_tasks[fut_idx] {
                    let waker = waker_data.to_waker();
                    let mut context = waker.make_context();

                    match fut_pointer.as_mut().poll(&mut context) {
                        core::task::Poll::Ready(()) => self.all_tasks[fut_idx] = None,
                        core::task::Poll::Pending => {}
                    }
                }
            }
        }

        pub fn all_tasks_done(&self) -> bool {
            self.all_tasks.iter().all(Option::is_none)
        }
    }
}

pub mod reactor {
    use core::{cell::RefCell, task::Waker, time::Duration};
    use std::io::{self, ErrorKind};

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
            TcpStreamWriteFut::new(self.reactor, self.socket_idx, to_write).await
        }
    }

    impl Drop for AsyncSocket<'_> {
        fn drop(&mut self) {
            self.reactor.close_connection(self.socket_idx);
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

        pub fn tick(&self) {
            let mut events = Events::with_capacity(128);
            let sockets = self.sockets.borrow();
            let mut poll = self.poll.borrow_mut();
            poll.poll(&mut events, Some(Duration::from_millis(1)))
                .unwrap();
            for event in &events {
                // find the socket we got an event for, and wake up that future
                let Token(socket_idx) = event.token();
                if let Some(waker) = sockets[socket_idx]
                    .borrow_mut()
                    .as_mut()
                    .expect("got event on a socket that we don't own??")
                    .waker
                    .take()
                {
                    waker.wake()
                }
            }
        }

        pub fn close_connection(&self, socket_idx: usize) {
            let sockets = self.sockets.borrow();
            let mut borrow = sockets[socket_idx].borrow_mut();
            if let Some(socket) = &mut *borrow {
                let result = socket.stream.shutdown(std::net::Shutdown::Both);
                match &result {
                    Ok(()) => (),
                    Err(e) => match e.kind() {
                        ErrorKind::NotConnected => (), // socket is already disconnected
                        _ => result.expect("why can't we shut down the socket?"),
                    },
                }
            }
            *borrow = None;
        }
    }
}

mod no_heap_waker {
    use core::{
        marker::PhantomData,
        task::{Context, RawWaker, RawWakerVTable, Waker},
    };
    use std::sync::mpsc::Sender;

    /// Represents a [Waker] where the data pointer in the [RawWaker] is actually a `&'a T`
    ///
    /// Since [Waker]/[RawWaker] don't have lifetimes, we put the lifetime on this wrapper struct
    pub struct LifetimedWaker<'a> {
        data_pointer_lifetime: PhantomData<&'a ()>,
        waker: Waker,
    }

    impl<'a> LifetimedWaker<'a> {
        fn new<T>(waker: RawWaker, _associated_lifetime: &'a T) -> Self {
            Self {
                data_pointer_lifetime: Default::default(),
                waker: unsafe { Waker::from_raw(waker) },
            }
        }
        pub fn make_context<'b>(&'b self) -> Context<'b> {
            Context::from_waker(&self.waker)
        }
    }

    #[derive(Clone)]
    pub struct WakerData {
        pub idx: usize,
        pub sender: Sender<usize>,
    }

    impl WakerData {
        fn wake_by_ref(&self) {
            self.sender.send(self.idx).unwrap()
        }

        /// Make a waker that refers to the data at this location
        pub fn to_waker(&self) -> LifetimedWaker {
            LifetimedWaker::new(
                RawWaker::new(self as *const _ as *const (), &REF_WAKER_VTABLE),
                self,
            )
        }
    }

    const REF_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
        |self_ptr| {
            // access through the same immutable reference
            RawWaker::new(self_ptr, &REF_WAKER_VTABLE)
        },
        |self_ptr| unsafe { (&*(self_ptr as *const WakerData)).wake_by_ref() },
        |self_ptr| unsafe { (&*(self_ptr as *const WakerData)).wake_by_ref() },
        |_self_ptr| {
            // no-op
            // we don't own self pointer, so dropping shouldn't do anything
        },
    );
}
