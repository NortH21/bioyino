use std::sync::atomic::Ordering;

use {DROPS, INGRESS};

use bytes::{BufMut, BytesMut};
use futures::sync::mpsc;
use futures::{Future, IntoFuture, Sink};
use tokio::executor::current_thread::spawn;
use tokio::net::UdpSocket;

use task::Task;

#[derive(Debug)]
pub struct StatsdServer {
    socket: UdpSocket,
    chans: Vec<mpsc::Sender<Task>>,
    buf: BytesMut,
    buf_queue_size: usize,
    bufsize: usize,
    next: usize,
    readbuf: BytesMut,
    chunks: usize,
}

impl StatsdServer {
    pub fn new(
        socket: UdpSocket,
        chans: Vec<mpsc::Sender<Task>>,
        buf: BytesMut,
        buf_queue_size: usize,
        bufsize: usize,
        next: usize,
        readbuf: BytesMut,
        chunks: usize,
    ) -> Self {
        Self {
            socket,
            chans,
            buf,
            buf_queue_size,
            bufsize,
            next,
            readbuf,
            chunks,
        }
    }
}

impl IntoFuture for StatsdServer {
    type Item = ();
    type Error = ();
    type Future = Box<Future<Item = Self::Item, Error = ()>>;

    fn into_future(self) -> Self::Future {
        let Self {
            socket,
            chans,
            mut buf,
            buf_queue_size,
            bufsize,
            next,
            readbuf,
            chunks,
        } = self;

        let future = socket
            .recv_dgram(readbuf)
            .map_err(|e| println!("error receiving UDP packet {:?}", e))
            .and_then(move |(socket, received, size, _addr)| {
                INGRESS.fetch_add(1, Ordering::Relaxed);
                if size == 0 {
                    return Ok(());
                }

                buf.put(&received[0..size]);

                if buf.remaining_mut() < bufsize || chunks == 0 {
                    let (chan, next) = if next >= chans.len() {
                        (chans[0].clone(), 1)
                    } else {
                        (chans[next].clone(), next + 1)
                    };
                    let newbuf = BytesMut::with_capacity(buf_queue_size * bufsize);

                    spawn(
                        chan.send(Task::Parse(buf.freeze()))
                            .map_err(|_| {
                                DROPS.fetch_add(1, Ordering::Relaxed);
                            })
                            .and_then(move |_| {
                                StatsdServer::new(
                                    socket,
                                    chans,
                                    newbuf,
                                    buf_queue_size,
                                    bufsize,
                                    next,
                                    received,
                                    buf_queue_size,
                                ).into_future()
                            }),
                    );
                } else {
                    spawn(
                        StatsdServer::new(
                            socket,
                            chans,
                            buf,
                            buf_queue_size,
                            bufsize,
                            next,
                            received,
                            chunks - 1,
                        ).into_future(),
                    );
                }
                Ok(())
            });
        Box::new(future)
    }
}
