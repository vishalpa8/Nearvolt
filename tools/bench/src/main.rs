//! Command-line tool for sending, receiving and stress-testing Nearvolt transfers.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::future::Future;
use std::io::{self, BufWriter, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use futures::FutureExt;
use nearvolt_core::util::AbortOnDrop;
use nearvolt_core::{
    human_bytes, send, Approver, Event, FileSet, Identity, Peer, ReceiveOptions, Receiver, Request, SendOptions, Stats,
    Summary, Ticket,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

const IDENTITY_PATH: &str = ".bench/identity.bin";

#[derive(Parser)]
#[command(name = "nearvolt-bench", version, about = "Send, receive and stress-test Nearvolt transfers")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Wait for files and print a ticket for the sender
    Receive {
        /// Folder to save received files in
        #[arg(long, default_value = "received")]
        dir: PathBuf,
        /// UDP port to listen on (0 picks a free one)
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Accept without asking
        #[arg(long)]
        yes: bool,
    },
    /// Send files or folders using a receiver's ticket
    Send {
        ticket: String,
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Parallel data connections
        #[arg(long, default_value_t = 8)]
        connections: usize,
    },
    /// Create incompressible test data: a few large files and many small ones
    Gen {
        dir: PathBuf,
        /// Total size of the large files, in GB
        #[arg(long, default_value_t = 2.0)]
        big_gb: f64,
        #[arg(long, default_value_t = 2)]
        big_files: u32,
        #[arg(long, default_value_t = 1000)]
        small_files: u32,
        /// Average size of a small file, in KB
        #[arg(long, default_value_t = 500)]
        small_kb: u64,
    },
    /// Check that two folders are byte-for-byte identical
    Compare { a: PathBuf, b: PathBuf },
    /// Send a folder to this same computer and verify it, optionally with simulated failures
    Local {
        source: PathBuf,
        /// Each run gets its own folder inside this one
        #[arg(long, default_value = ".bench/runs")]
        dest: PathBuf,
        /// Parallel data connections
        #[arg(long, default_value_t = 8)]
        connections: usize,
        /// Cut the link (stop all traffic) once this many MB have been sent
        #[arg(long)]
        drop_link_at_mb: Option<u64>,
        /// How long the link stays cut, in seconds
        #[arg(long, default_value_t = 6.0)]
        drop_link_secs: f64,
        /// Kill both apps once this many MB have been sent, then start them again
        #[arg(long)]
        kill_at_mb: Option<u64>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Receive { dir, port, yes } => receive(dir, port, yes).await,
        Command::Send { ticket, paths, connections } => send_files(&ticket, paths, connections).await,
        Command::Gen { dir, big_gb, big_files, small_files, small_kb } => {
            tokio::task::spawn_blocking(move || generate(&dir, big_gb, big_files, small_files, small_kb)).await?
        }
        Command::Compare { a, b } => tokio::task::spawn_blocking(move || compare(&a, &b)).await?,
        Command::Local { source, dest, connections, drop_link_at_mb, drop_link_secs, kill_at_mb } => {
            local(source, dest, connections, drop_link_at_mb, drop_link_secs, kill_at_mb).await
        }
    }
}

async fn receive(dir: PathBuf, port: u16, yes: bool) -> Result<()> {
    let identity = Identity::load_or_create(Path::new(IDENTITY_PATH))?;
    let options =
        ReceiveOptions { dir: dir.clone(), journal_dir: None, device_name: device_name(), allow_code_pairing: false };
    let receiver = Receiver::bind(&identity, SocketAddr::from(([0, 0, 0, 0], port)), options)?;
    println!("Ready to receive into {}", dir.display());
    for addr in &receiver.ticket().addrs {
        println!("  reachable at {addr}");
    }
    println!("\nTicket for the sender:\n\n  {}\n", receiver.ticket().encode());

    let approve: Approver =
        if yes { Arc::new(|_| async { true }.boxed()) } else { Arc::new(|request| ask(request).boxed()) };
    let stats = Arc::new(Stats::default());
    let (tx, rx) = mpsc::unbounded_channel();
    let summary = with_progress("Receiving", stats.clone(), rx, receiver.run(approve, stats.clone(), Some(tx))).await?;
    print_summary("Received", &summary);
    Ok(())
}

async fn ask(request: Request) -> bool {
    println!("\n{} wants to connect.", request.sender_name);
    if let Some(code) = &request.code {
        println!("Check that it shows the code {code}.");
    }
    print!("Accept? [y/N] ");
    let _ = io::stdout().flush();
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);
        line.trim().eq_ignore_ascii_case("y")
    })
    .await
    .unwrap_or(false)
}

async fn send_files(ticket: &str, paths: Vec<PathBuf>, connections: usize) -> Result<()> {
    let ticket = Ticket::decode(ticket)?;
    let identity = Identity::load_or_create(Path::new(IDENTITY_PATH))?;
    let options = SendOptions { device_name: device_name(), connections, ..Default::default() };
    let stats = Arc::new(Stats::default());
    let (tx, rx) = mpsc::unbounded_channel();
    println!("Sending to {}", ticket.name);
    let peer = Peer::from_ticket(&ticket);
    let task = send(&identity, &peer, FileSet::Paths(paths), &options, stats.clone(), Some(tx));
    let summary = with_progress("Sending", stats, rx, task).await?;
    print_summary("Sent", &summary);
    Ok(())
}

/// Runs a transfer against this same computer, then checks every byte.
async fn local(
    source: PathBuf,
    dest: PathBuf,
    connections: usize,
    drop_link_at_mb: Option<u64>,
    drop_link_secs: f64,
    kill_at_mb: Option<u64>,
) -> Result<()> {
    let source = fs::canonicalize(&source).with_context(|| format!("can't find {}", source.display()))?;
    let name = source.file_name().context("pick a folder, not a drive root")?.to_owned();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let run_dir = dest.join(format!("run-{stamp}"));
    fs::create_dir_all(&run_dir)?;

    let receiver_id = Identity::generate()?;
    let sender_id = Identity::generate()?;
    let options = SendOptions { device_name: "Sender".into(), connections, ..Default::default() };
    let tx_stats = Arc::new(Stats::default());
    let blackout = Arc::new(AtomicBool::new(false));
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (mut dropped, mut killed) = (false, false);
    let started = Instant::now();
    println!("Sending {} to {} over this computer's loopback", source.display(), run_dir.display());

    let summary = 'run: loop {
        let receiver = Receiver::bind(
            &receiver_id,
            SocketAddr::from(([127, 0, 0, 1], 0)),
            ReceiveOptions {
                dir: run_dir.clone(),
                journal_dir: None,
                device_name: "Receiver".into(),
                allow_code_pairing: false,
            },
        )?;
        let mut ticket = receiver.ticket();
        let _proxy = match drop_link_at_mb {
            Some(_) if !dropped => {
                let (addr, proxy) = start_proxy(receiver.local_addr(), blackout.clone()).await?;
                ticket.addrs = vec![addr];
                Some(proxy)
            }
            _ => None,
        };

        let mut receive_task = AbortOnDrop(tokio::spawn(async move {
            let approve: Approver = Arc::new(|_| async { true }.boxed());
            receiver.run(approve, Arc::new(Stats::default()), None).await
        }));
        let mut send_task = AbortOnDrop(tokio::spawn({
            let (id, stats, options, events) =
                (sender_id.clone(), tx_stats.clone(), options.clone(), events_tx.clone());
            let paths = vec![source.clone()];
            async move { send(&id, &Peer::from_ticket(&ticket), FileSet::Paths(paths), &options, stats, Some(events)).await }
        }));

        let mut meter = Meter::default();
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        loop {
            tokio::select! {
                result = &mut send_task.0 => {
                    meter.print("Sending", &tx_stats);
                    println!();
                    let summary = result??;
                    (&mut receive_task.0).await??;
                    break 'run summary;
                }
                Some(event) = events_rx.recv() => print_event(event),
                _ = tick.tick() => {
                    meter.print("Sending", &tx_stats);
                    let sent_mb = tx_stats.snapshot().done_bytes / 1_000_000;
                    if drop_link_at_mb.is_some_and(|at| !dropped && sent_mb >= at) {
                        dropped = true;
                        blackout.store(true, Ordering::Relaxed);
                        note(&format!("Link cut at {sent_mb} MB: all traffic stopped for {drop_link_secs} s"));
                        let blackout = blackout.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs_f64(drop_link_secs)).await;
                            blackout.store(false, Ordering::Relaxed);
                            note("Link restored");
                        });
                    }
                    if kill_at_mb.is_some_and(|at| !killed && sent_mb >= at) {
                        killed = true;
                        note(&format!("Killed both apps at {sent_mb} MB. Starting them again"));
                        drop(send_task);
                        drop(receive_task);
                        // Let in-flight disk writes from the killed receiver finish.
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        continue 'run;
                    }
                }
            }
        }
    };

    let elapsed = started.elapsed();
    println!();
    println!(
        "Transfer finished: {} files, {} in {:.1} s ({}/s average, including any outage)",
        summary.files,
        human_bytes(summary.bytes),
        elapsed.as_secs_f64(),
        human_bytes((summary.bytes as f64 / elapsed.as_secs_f64()) as u64)
    );
    if run_dir.join(".nearvolt").exists() {
        bail!("the receiver left its journal behind");
    }
    let received = run_dir.join(&name);
    tokio::task::spawn_blocking(move || compare(&source, &received)).await?
}

async fn with_progress<T>(
    label: &str,
    stats: Arc<Stats>,
    mut events: mpsc::UnboundedReceiver<Event>,
    task: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::pin!(task);
    let mut meter = Meter::default();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            result = &mut task => {
                meter.print(label, &stats);
                println!();
                return result;
            }
            Some(event) = events.recv() => print_event(event),
            _ = tick.tick() => meter.print(label, &stats),
        }
    }
}

#[derive(Default)]
struct Meter {
    last: Option<(Instant, u64)>,
    speed: f64,
}

impl Meter {
    fn print(&mut self, label: &str, stats: &Stats) {
        let p = stats.snapshot();
        let now = Instant::now();
        if let Some((at, bytes)) = self.last {
            let secs = now.duration_since(at).as_secs_f64();
            if secs > 0.0 {
                let instant = p.done_bytes.saturating_sub(bytes) as f64 / secs;
                self.speed = if self.speed == 0.0 { instant } else { self.speed * 0.7 + instant * 0.3 };
            }
        }
        self.last = Some((now, p.done_bytes));
        let percent = if p.total_bytes == 0 { 100.0 } else { p.done_bytes as f64 * 100.0 / p.total_bytes as f64 };
        let eta = if self.speed > 1.0 {
            let secs = (p.total_bytes.saturating_sub(p.done_bytes) as f64 / self.speed) as u64;
            format!("{}:{:02}", secs / 60, secs % 60)
        } else {
            "-".into()
        };
        print!(
            "\r{label} {percent:5.1}%  {} of {}  {}/s  {}/{} files  ETA {eta}     ",
            human_bytes(p.done_bytes),
            human_bytes(p.total_bytes),
            human_bytes(self.speed as u64),
            p.done_files,
            p.total_files,
        );
        let _ = io::stdout().flush();
    }
}

fn print_event(event: Event) {
    match event {
        Event::Connected { peer } => note(&format!("Connected to {peer}")),
        Event::Code { code } => note(&format!("Pairing code {code}: check the receiver shows the same")),
        Event::ConnectionLost { reason } => note(&format!("Connection lost ({reason}). Reconnecting")),
        Event::BatchFailed { reason, .. } => note(&format!("Batch refused: {reason}")),
        Event::PeerLeft => note("The other side disconnected"),
        Event::Started { .. } | Event::Incoming { .. } | Event::Completed { .. } => {}
    }
}

fn note(text: &str) {
    println!("\r>> {text:<100}");
}

fn print_summary(verb: &str, s: &Summary) {
    let secs = s.elapsed.as_secs_f64().max(0.001);
    println!(
        "{verb} {} files ({}) in {:.1} s, {}/s average",
        s.files,
        human_bytes(s.bytes),
        secs,
        human_bytes((s.bytes as f64 / secs) as u64)
    );
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "This computer".into())
}

/// A TCP relay between sender and receiver that can stop all traffic on demand,
/// to simulate walking out of range. Connections stay open but go silent, exactly
/// like a radio link that has faded.
async fn start_proxy(target: SocketAddr, blackout: Arc<AtomicBool>) -> Result<(SocketAddr, AbortOnDrop<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let mut pipes = JoinSet::new();
        loop {
            let Ok((client, _)) = listener.accept().await else { continue };
            let blackout = blackout.clone();
            pipes.spawn(async move {
                let Ok(server) = TcpStream::connect(target).await else { return };
                let _ = (client.set_nodelay(true), server.set_nodelay(true));
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                tokio::join!(
                    pump(client_read, server_write, blackout.clone()),
                    pump(server_read, client_write, blackout),
                );
            });
        }
    });
    Ok((addr, AbortOnDrop(task)))
}

async fn pump(mut from: OwnedReadHalf, mut to: OwnedWriteHalf, blackout: Arc<AtomicBool>) {
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        while blackout.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

fn generate(dir: &Path, big_gb: f64, big_files: u32, small_files: u32, small_kb: u64) -> Result<()> {
    fs::create_dir_all(dir)?;
    let started = Instant::now();
    let mut total = 0u64;
    if big_files > 0 {
        let each = (big_gb * 1e9) as u64 / big_files as u64;
        for i in 0..big_files {
            write_random(&dir.join(format!("video-{i:02}.bin")), each, i as u64)?;
            total += each;
        }
    }
    let avg = small_kb * 1000;
    for i in 0..small_files {
        let folder = dir.join("photos").join(format!("album-{:03}", i / 100));
        fs::create_dir_all(&folder)?;
        // Sizes spread between half and one and a half times the average.
        let size = avg / 2 + (u64::from(i) * 7919) % (avg + 1);
        write_random(&folder.join(format!("IMG_{i:05}.jpg")), size, 1_000_000 + u64::from(i))?;
        total += size;
    }
    fs::create_dir_all(dir.join("notes"))?;
    File::create(dir.join("notes").join("empty.txt"))?;
    println!("Created {} in {} ({:.1} s)", human_bytes(total), dir.display(), started.elapsed().as_secs_f64());
    Ok(())
}

fn write_random(path: &Path, size: u64, seed: u64) -> io::Result<()> {
    let mut xof = blake3::Hasher::new().update(&seed.to_le_bytes()).finalize_xof();
    let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let mut buf = vec![0u8; 1 << 20];
    let mut left = size;
    while left > 0 {
        let n = left.min(buf.len() as u64) as usize;
        xof.fill(&mut buf[..n]);
        out.write_all(&buf[..n])?;
        left -= n as u64;
    }
    out.flush()
}

fn compare(a: &Path, b: &Path) -> Result<()> {
    let left = list_files(a)?;
    let right = list_files(b)?;
    let missing: Vec<_> = left.keys().filter(|k| !right.contains_key(*k)).take(5).collect();
    let extra: Vec<_> = right.keys().filter(|k| !left.contains_key(*k)).take(5).collect();
    if !missing.is_empty() || !extra.is_empty() {
        bail!("the folders differ. Missing: {missing:?}. Unexpected: {extra:?}");
    }
    let pairs: Vec<(String, PathBuf, PathBuf)> = left
        .into_iter()
        .map(|(rel, pa)| {
            let pb = right[&rel].clone();
            (rel, pa, pb)
        })
        .collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let per = pairs.len().div_ceil(threads).max(1);
    let bytes = std::thread::scope(|scope| -> Result<u64> {
        let handles: Vec<_> = pairs
            .chunks(per)
            .map(|group| {
                scope.spawn(move || -> Result<u64> {
                    let mut bytes = 0;
                    for (rel, pa, pb) in group {
                        let (ha, sa) = hash_file(pa)?;
                        let (hb, sb) = hash_file(pb)?;
                        if sa != sb || ha != hb {
                            bail!("{rel} is different");
                        }
                        bytes += sa;
                    }
                    Ok(bytes)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("hash thread panicked")).sum()
    })?;
    println!("Verified identical: {} files, {} (every byte checked with BLAKE3)", pairs.len(), human_bytes(bytes));
    Ok(())
}

fn list_files(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("can't read {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if entry.file_type()?.is_dir() {
                stack.push((entry.path(), child));
            } else {
                files.insert(child, entry.path());
            }
        }
    }
    Ok(files)
}

fn hash_file(path: &Path) -> Result<(blake3::Hash, u64)> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(File::open(path).with_context(|| format!("can't read {}", path.display()))?)?;
    Ok((hasher.finalize(), fs::metadata(path)?.len()))
}
