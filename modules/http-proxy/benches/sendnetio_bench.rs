use criterion::{criterion_group, criterion_main, Criterion};
use std::net::{TcpListener, TcpStream as StdTcpStream};
use std::thread;
use vibeio::net::TcpStream as VibeTcpStream;

use ferron_http_proxy::SendTcpStreamPoll;

fn bench_sendtcp_new(c: &mut Criterion) {
    c.bench_function("sendtcp_new_comp_io", |b| {
        b.iter(|| {
            // Bind ephemeral listener and connect a std TcpStream to it
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().unwrap();
            let client = thread::spawn(move || {
                let _ = StdTcpStream::connect(addr).expect("connect");
            });
            let (accepted, _peer) = listener.accept().expect("accept");
            client.join().expect("client join");

            // Convert to vibeio TcpStream and create SendTcpStreamPoll
            let vibe = VibeTcpStream::from_std(accepted).expect("from_std");
            let _wrap = SendTcpStreamPoll::new_comp_io(vibe).expect("new_comp_io");
        });
    });
}

criterion_group!(benches, bench_sendtcp_new);
criterion_main!(benches);
