#![allow(dead_code)]

use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    io::{self, IoSliceMut},
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use parking_lot::Mutex;
use rand::Rng;
use smallvec::ToSmallVec;

/// A QUIC datagram together with the receive metadata produced by the
/// underlying socket. The metadata (source address, ECN, destination IP) must
/// travel with the bytes, otherwise a datagram routed to a different endpoint
/// would lose the information quinn needs to answer it.
#[derive(Debug, Clone)]
struct QuinnMTDatagram {
    data: smallvec::SmallVec<[u8; 1536]>,
    meta: quinn::udp::RecvMeta,
}

/// A runtime that fans a single UDP socket out to several independent quinn
/// endpoints. Each endpoint gets its own [`QuinnMTUdpSocket`] that wraps the
/// same underlying socket and routes every incoming datagram to the endpoint
/// that owns the connection, chosen by hashing the destination connection ID.
///
/// For routing to stay consistent across a connection's lifetime the endpoints
/// must use [`QuinnMTConnectionIdGenerator`], which only issues connection IDs
/// that hash back to the endpoint that generated them.
#[derive(Debug)]
pub struct QuinnMTRuntime<Rt> {
    inner: Rt,
    channels: Arc<QuinnMTChannels>,
    /// Stable index of this endpoint in the shared channel table.
    id: usize,
}

/// Shared routing table. One slot per endpoint, indexed by its `id`. Each
/// endpoint registers its sender here when its socket is created.
#[derive(Debug)]
pub struct QuinnMTChannels {
    inner: Mutex<Vec<Option<Arc<QuinnMTDatagramQueue>>>>,
    /// Length of the server connection IDs used for routing. It must match the
    /// connection ID length the endpoints issue, because short-header packets
    /// carry no length field and the router has to know where the ID ends.
    cid_len: usize,
}

impl QuinnMTChannels {
    /// `endpoint_count` is the number of endpoints sharing the socket; `cid_len`
    /// is the server connection ID length they will issue (8 by default).
    #[inline]
    pub fn new(endpoint_count: usize, cid_len: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(vec![None; endpoint_count]),
            cid_len: if cid_len == 0 { 8 } else { cid_len },
        })
    }

    #[inline]
    fn register(&self, id: usize, queue: Arc<QuinnMTDatagramQueue>) {
        let mut guard = self.inner.lock();
        guard[id] = Some(queue);
    }

    /// Map a connection ID to the endpoint that should handle it.
    ///
    /// Returns `None` when the datagram belongs to the endpoint calling this
    /// method (`inner_id`), or `Some(sender)` for the owning endpoint.
    #[inline]
    fn select(&self, conn_id: &[u8], inner_id: usize) -> Option<Arc<QuinnMTDatagramQueue>> {
        let guard = self.inner.lock();
        let len = guard.len();
        if len == 0 {
            return None;
        }
        let mut hasher = rustc_hash::FxHasher::default();
        conn_id.hash(&mut hasher);
        let selected = (hasher.finish() as usize) % len;
        if selected == inner_id {
            None
        } else {
            guard[selected].clone()
        }
    }
}

impl<Rt: quinn::Runtime> QuinnMTRuntime<Rt> {
    /// Build a runtime sharing `channels` with other endpoints. `id` is this
    /// endpoint's stable index (0..endpoint_count); pass the same `channels`
    /// `Arc` to every endpoint.
    #[inline]
    pub fn new(inner: Rt, channels: Arc<QuinnMTChannels>, id: usize) -> Self {
        Self {
            inner,
            channels,
            id,
        }
    }

    /// Build the connection ID generator this endpoint must use on its
    /// `EndpointConfig`, so the CIDs it issues route back to this endpoint.
    #[inline]
    pub fn cid_generator(&self) -> QuinnMTConnectionIdGenerator {
        QuinnMTConnectionIdGenerator::new(self.id, self.channels.clone())
    }
}

impl<Rt: quinn::Runtime + std::fmt::Debug> quinn::Runtime for QuinnMTRuntime<Rt> {
    #[inline]
    fn new_timer(&self, i: std::time::Instant) -> Pin<Box<dyn quinn::AsyncTimer>> {
        self.inner.new_timer(i)
    }

    #[inline]
    fn now(&self) -> std::time::Instant {
        self.inner.now()
    }

    #[inline]
    fn spawn(&self, future: Pin<Box<dyn std::future::Future<Output = ()> + Send>>) {
        self.inner.spawn(future)
    }

    #[inline]
    fn wrap_udp_socket(
        &self,
        t: std::net::UdpSocket,
    ) -> io::Result<Arc<dyn quinn::AsyncUdpSocket>> {
        let inner = self.inner.wrap_udp_socket(t)?;
        let queue = Arc::new(QuinnMTDatagramQueue::new());
        self.channels.register(self.id, queue.clone());
        Ok(Arc::new(QuinnMTUdpSocket {
            inner,
            channels: self.channels.clone(),
            queue,
            id: self.id,
        }))
    }
}

#[derive(Debug)]
struct QuinnMTUdpSocket {
    inner: Arc<dyn quinn::AsyncUdpSocket>,
    channels: Arc<QuinnMTChannels>,
    queue: Arc<QuinnMTDatagramQueue>,
    /// Index of this endpoint in the shared channel table.
    id: usize,
}

impl QuinnMTUdpSocket {
    /// Pull a datagram that another endpoint forwarded to us and copy it into
    /// the caller's buffers.
    #[inline]
    fn deliver(
        &self,
        dgram: QuinnMTDatagram,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [quinn::udp::RecvMeta],
    ) -> usize {
        let data = &dgram.data;
        let len = data.len().min(bufs[0].len());
        bufs[0][..len].copy_from_slice(&data[..len]);
        let mut m = dgram.meta;
        m.len = len;
        meta[0] = m;
        1
    }
}

impl quinn::AsyncUdpSocket for QuinnMTUdpSocket {
    #[inline]
    fn max_receive_segments(&self) -> usize {
        self.inner.max_receive_segments()
    }

    #[inline]
    fn max_transmit_segments(&self) -> usize {
        self.inner.max_transmit_segments()
    }

    #[inline]
    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }

    #[inline]
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn quinn::UdpPoller>> {
        self.inner.clone().create_io_poller()
    }

    #[inline]
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    #[inline]
    fn try_send(&self, transmit: &quinn::udp::Transmit) -> io::Result<()> {
        self.inner.try_send(transmit)
    }

    #[inline]
    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [quinn::udp::RecvMeta],
    ) -> Poll<io::Result<usize>> {
        loop {
            // 1. First serve datagrams other endpoints forwarded to us.
            //
            // Could have used `kanal`, but using that would cause too much packet loss...
            if let Some(dgram) = self.queue.try_recv() {
                return Poll::Ready(Ok(self.deliver(dgram, bufs, meta)));
            }

            // 2. Receive a batch from the shared underlying socket. A batch may
            //    carry several datagrams (GRO) for different endpoints; route
            //    each one to the endpoint that owns it.
            let n = match self.inner.poll_recv(cx, bufs, meta) {
                Poll::Ready(Ok(n)) => n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => {
                    // Register a waker on the channel so a datagram forwarded to
                    // us wakes this task even when no new data arrives on the
                    // underlying socket. `poll_recv` keeps the waker registered
                    // (unlike a one-shot future), so it is safe to drop here.
                    if let Poll::Ready(dgram) = self.queue.poll_recv(cx) {
                        return Poll::Ready(Ok(self.deliver(dgram, bufs, meta)));
                    }
                    return Poll::Pending;
                }
            };

            // Walk every segment in the batch. Segments for other endpoints are
            // forwarded; segments for this endpoint are compacted to the front of
            // the buffer so we can return them all in one `poll_recv` call.
            let cid_len = self.channels.cid_len;
            let mut keep = 0;
            for src in 0..n {
                let len = meta[src].len;
                let packet = &bufs[src][..len];
                // `None` means the packet is unparseable: keep it locally rather
                // than drop it, so this endpoint can reject it itself.
                let routed = match extract_conn_id(packet, cid_len) {
                    None => None,
                    Some(cid) => self.channels.select(cid, self.id),
                };
                match routed {
                    // Belongs to this endpoint (or is unparseable): keep it.
                    None => {
                        // `keep < src` always holds here, so splitting at `src`
                        // lets us copy from `src` into `keep` without aliasing
                        // the same mutable borrow of `bufs`.
                        if keep != src {
                            let (left, right) = bufs.split_at_mut(src);
                            left[keep][..len].copy_from_slice(&right[0][..len]);
                            meta[keep] = meta[src];
                        }
                        keep += 1;
                    }
                    // Belongs to another endpoint: forward the whole segment.
                    Some(target) => {
                        let data = bufs[src][..len].to_smallvec();
                        let m = meta[src];
                        target.send(QuinnMTDatagram { data, meta: m });
                    }
                }
            }

            if keep == 0 {
                // Every datagram in this batch belongs to another endpoint. The
                // batch is already consumed from the socket, so loop to receive
                // the next one rather than re-reading stale buffers.
                continue;
            }
            return Poll::Ready(Ok(keep));
        }
    }
}

/// Connection ID generator that keeps a connection's packets on the endpoint
/// that owns it.
///
/// quinn generates a fresh local connection ID per connection and uses it as the
/// destination connection ID on every packet the client sends afterwards. This
/// generator only returns IDs whose hash (using the same function and modulus
/// as [`QuinnMTChannels::select`]) equals `id`, so all of a connection's
/// packets route back to this endpoint regardless of the random connection ID
/// the client chose for its first Initial.
pub struct QuinnMTConnectionIdGenerator {
    id: usize,
    channels: Arc<QuinnMTChannels>,
}

impl QuinnMTConnectionIdGenerator {
    #[inline]
    pub fn new(id: usize, channels: Arc<QuinnMTChannels>) -> Self {
        Self { id, channels }
    }
}

impl quinn::ConnectionIdGenerator for QuinnMTConnectionIdGenerator {
    #[inline]
    fn generate_cid(&mut self) -> quinn::ConnectionId {
        let len = {
            let guard = self.channels.inner.lock();
            guard.len().max(1)
        };
        let cid_len = self.channels.cid_len;
        let mut bytes: smallvec::SmallVec<[u8; 8]> = smallvec::smallvec![0u8; cid_len];
        loop {
            rand::rng().fill_bytes(&mut bytes);
            let mut hasher = rustc_hash::FxHasher::default();
            bytes.hash(&mut hasher);
            if (hasher.finish() as usize) % len == self.id {
                return quinn::ConnectionId::new(&bytes);
            }
        }
    }

    #[inline]
    fn cid_len(&self) -> usize {
        self.channels.cid_len
    }

    #[inline]
    fn cid_lifetime(&self) -> Option<Duration> {
        None
    }
}

/// Extract the connection ID used for routing from a raw QUIC packet.
///
/// For long headers the destination connection ID length is encoded in the
/// packet, so it is read directly. For short headers the connection ID has no
/// length prefix; the server-issued length (`cid_len`) is used instead.
///
/// The destination connection ID is the server's chosen connection ID, which is
/// the stable key that maps an incoming packet to the endpoint that owns the
/// connection, and is present in both long and short header packets.
#[inline]
fn extract_conn_id(packet: &[u8], cid_len: usize) -> Option<&[u8]> {
    let &first = packet.first()?;
    if first & 0x80 != 0 {
        // Long header: flag (1) + version (4) + DCID len (1) + DCID ...
        if packet.len() < 6 {
            return None;
        }
        let dcid_len = packet[5] as usize;
        let off = 6;
        if packet.len() < off + dcid_len {
            return None;
        }
        Some(&packet[off..off + dcid_len])
    } else {
        // Short header: flag (1) + DCID (cid_len) ...
        if packet.len() < 1 + cid_len {
            return None;
        }
        Some(&packet[1..1 + cid_len])
    }
}

/// A queue of [`QuinnMTDatagram`]s, used for buffering incoming datagrams before they
/// are delivered to the application.
///
/// This queue also supports wakers so that it can be used in poll functions of Futures.
#[derive(Debug)]
struct QuinnMTDatagramQueue {
    // Here, Mutex is fine, since QuinnMTRuntime is supposed to be per-thread
    inner: Mutex<VecDeque<QuinnMTDatagram>>,
    wakers: Mutex<smallvec::SmallVec<[Waker; 8]>>,
}

impl QuinnMTDatagramQueue {
    #[inline]
    fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn try_recv(&self) -> Option<QuinnMTDatagram> {
        let mut inner = self.inner.lock();
        inner.pop_front()
    }

    #[inline]
    fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<QuinnMTDatagram> {
        let mut inner = self.inner.lock();
        if let Some(datagram) = inner.pop_front() {
            Poll::Ready(datagram)
        } else {
            self.wakers.lock().push(cx.waker().clone());
            Poll::Pending
        }
    }

    #[inline]
    fn send(&self, datagram: QuinnMTDatagram) {
        let mut inner = self.inner.lock();
        inner.push_back(datagram);
        for waker in self.wakers.lock().drain(..) {
            waker.wake();
        }
    }
}

impl Default for QuinnMTDatagramQueue {
    #[inline]
    fn default() -> Self {
        Self {
            inner: parking_lot::Mutex::new(VecDeque::with_capacity(1024)),
            wakers: parking_lot::Mutex::new(smallvec::SmallVec::new()),
        }
    }
}
