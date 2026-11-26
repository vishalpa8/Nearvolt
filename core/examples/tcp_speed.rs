//! Plain TCP throughput over loopback, as a baseline for the QUIC transport.
//!
//!   cargo run --release --example tcp_speed -- [gigabytes] [streams]

use std::time::Instant;

use anyhow::Result;
use proxima_core::human_bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let gigabytes: f64 = args.next().map_or(Ok(2.0), |a| a.parse())?;
    let streams: usize = args.next().map_or(Ok(8), |a| a.parse())?;
    let per_stream = (gigabytes * 1e9) as u64 / streams as u64;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let sink = tokio::spawn(async move {
        let mut readers = Vec::new();
        for _ in 0..streams {
            let (mut socket, _) = listener.accept().await.unwrap();
            readers.push(tokio::spawn(async move {
                let mut buf = vec![0u8; 1 << 20];
                let mut n = 0u64;
                loop {
                    let read = socket.read(&mut buf).await.unwrap();
                    if read == 0 {
                        break n;
                    }
                    n += read as u64;
                }
            }));
        }
        let mut received = 0;
        for r in readers {
            received += r.await.unwrap();
        }
        received
    });

    let started = Instant::now();
    let chunk = vec![7u8; 4 << 20];
    let mut writers = Vec::new();
    for _ in 0..streams {
        let mut socket = TcpStream::connect(addr).await?;
        let chunk = chunk.clone();
        writers.push(tokio::spawn(async move {
            let mut left = per_stream;
            while left > 0 {
                let n = left.min(chunk.len() as u64) as usize;
                socket.write_all(&chunk[..n]).await.unwrap();
                left -= n as u64;
            }
            socket.shutdown().await.unwrap();
        }));
    }
    for w in writers {
        w.await?;
    }
    let received = sink.await?;
    let secs = started.elapsed().as_secs_f64();
    println!(
        "{} over {streams} TCP streams in {secs:.2} s = {}/s ({:.2} Gbit/s)",
        human_bytes(received),
        human_bytes((received as f64 / secs) as u64),
        received as f64 * 8.0 / secs / 1e9
    );
    Ok(())
}
