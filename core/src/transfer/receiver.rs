//! Receiving side: accepts authenticated senders, verifies and writes chunks, and
//! journals progress so interrupted transfers resume where they stopped.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::future::BoxFuture;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

use super::disk;
use super::journal::{Journal, Replay};
use super::{emit, human_bytes, is_fatal, Event, EventSender, Fatal, Stats, Summary};
use crate::identity::{Fingerprint, Identity};
use crate::pairing::{self, constant_time_eq, truncate_utf8, Secret, Ticket, MAX_NAME_LEN};
use crate::protocol::{
    bit_clear, bit_get, bit_set, bitmap_len, chunk_count, chunk_range, read_header, read_msg, write_msg,
    ChunkHeader, ChunkRef, Control, FileHave, Offer, SessionId, CHUNK_HEADER_LEN, HEARTBEAT_INTERVAL,
    HEARTBEAT_TIMEOUT, MAX_CHUNK_SIZE, MAX_FILES, MIN_CHUNK_SIZE, PROTOCOL_VERSION,
};
use crate::safety;
use crate::transport::{Acceptor, Listener, TlsStream};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CHECKPOINT_EVERY: Duration = Duration::from_secs(2);
const MAX_AUTH_FAILURES: u32 = 20;
const PART_SUFFIX: &str = ".pxpart";
const STATE_DIR: &str = ".proxima";

#[derive(Clone, Debug)]
pub struct ReceiveOptions {
    /// Destination folder.
    pub dir: PathBuf,
    pub device_name: String,
    /// Re-read every finished file from disk and check it against the chunk hashes.
    pub verify_on_disk: bool,
}

#[derive(Clone, Debug)]
pub struct IncomingOffer {
    pub sender_name: String,
    pub files: u64,
    pub bytes: u64,
    /// The first few paths, for an approval prompt.
    pub sample: Vec<String>,
}

/// Decides whether to accept an offer, for example by asking the user.
pub type Approver = Arc<dyn Fn(IncomingOffer) -> BoxFuture<'static, bool> + Send + Sync>;

pub struct Receiver {
    listener: Listener,
    ticket: Ticket,
    options: Arc<ReceiveOptions>,
}

impl Receiver {
    pub fn bind(identity: &Identity, bind: SocketAddr, options: ReceiveOptions) -> Result<Self> {
        let listener = Listener::bind(identity, bind)?;
        let port = listener.local_addr()?.port();
        let addrs = if bind.ip().is_unspecified() {
            pairing::local_addrs(port)
        } else {
            vec![SocketAddr::new(bind.ip(), port)]
        };
        let ticket = Ticket {
            fingerprint: *identity.fingerprint(),
            secret: pairing::new_secret(),
            addrs,
            name: options.device_name.clone(),
        };
        Ok(Self { listener, ticket, options: Arc::new(options) })
    }

    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Receives one transfer, across as many reconnects as it takes.
    pub async fn run(&self, approve: Approver, stats: Arc<Stats>, events: Option<EventSender>) -> Result<Summary> {
        fs::create_dir_all(&self.options.dir)
            .with_context(|| format!("can't create {}", self.options.dir.display()))?;
        let shared = Arc::new(Shared {
            options: self.options.clone(),
            approve,
            stats,
            transfer: tokio::sync::Mutex::new(None),
        });
        let (auth_tx, mut auth_rx) = mpsc::channel::<Option<Authed>>(32);
        let mut current: Option<Current> = None;
        let mut failures = 0;
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let Ok((tcp, acceptor)) = accepted else { continue };
                    let (tx, secret) = (auth_tx.clone(), self.ticket.secret);
                    tokio::spawn(async move {
                        let _ = tx.send(authenticate(tcp, acceptor, secret).await.ok()).await;
                    });
                }
                Some(authed) = auth_rx.recv() => {
                    let Some(authed) = authed else {
                        failures += 1;
                        if failures >= MAX_AUTH_FAILURES {
                            return Err(Fatal("Too many failed pairing attempts. Start a new receive session.".into()).into());
                        }
                        continue;
                    };
                    match authed.session {
                        Some(session) => {
                            if let Some(c) = current.as_ref().filter(|c| c.session == session && c.sender == authed.sender) {
                                let _ = c.data_tx.try_send(authed.stream);
                            }
                        }
                        None => {
                            // A reconnecting sender replaces its stale session at once
                            // instead of waiting for the old one to time out.
                            drop(current.take());
                            emit(&events, Event::Connected { peer: authed.sender_name.clone() });
                            let (session, sender) = (pairing::new_secret(), authed.sender);
                            let (data_tx, data_rx) = mpsc::channel(64);
                            let task = AbortOnDrop(tokio::spawn(serve(shared.clone(), authed, session, data_rx)));
                            current = Some(Current { session, sender, data_tx, task });
                        }
                    }
                }
                result = join(&mut current), if current.is_some() => {
                    current = None;
                    match result {
                        Ok(Outcome::Completed(summary)) => {
                            emit(&events, Event::Completed(summary));
                            return Ok(summary);
                        }
                        Ok(Outcome::Declined) => return Err(Fatal("The transfer was declined.".into()).into()),
                        Ok(Outcome::Busy) => {}
                        Err(e) if is_fatal(&e) => return Err(e),
                        Err(e) => emit(&events, Event::ConnectionLost { reason: format!("{e:#}") }),
                    }
                }
            }
        }
    }
}

struct Shared {
    options: Arc<ReceiveOptions>,
    approve: Approver,
    stats: Arc<Stats>,
    transfer: tokio::sync::Mutex<Option<Arc<TransferState>>>,
}

enum Outcome {
    Completed(Summary),
    Declined,
    Busy,
}

struct Authed {
    stream: TlsStream,
    sender: Fingerprint,
    sender_name: String,
    /// `None` for a control connection; the session it belongs to for a data connection.
    session: Option<SessionId>,
}

/// The session currently being served. Data connections join it by session ID.
struct Current {
    session: SessionId,
    sender: Fingerprint,
    data_tx: mpsc::Sender<TlsStream>,
    task: AbortOnDrop<Result<Outcome>>,
}

struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn join(current: &mut Option<Current>) -> Result<Outcome> {
    match current {
        Some(c) => (&mut c.task.0).await.unwrap_or_else(|e| Err(anyhow!("connection task failed: {e}"))),
        None => std::future::pending().await,
    }
}

async fn authenticate(tcp: TcpStream, acceptor: Acceptor, secret: Secret) -> Result<Authed> {
    let (mut stream, sender) = acceptor.handshake(tcp).await?;
    let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_msg(&mut stream)).await??;
    let Control::Hello { version, secret: offered, device_name, session } = hello else {
        bail!("expected hello");
    };
    if !constant_time_eq(&offered, &secret) {
        let _ = write_msg(&mut stream, &reject("This code is not valid. Scan the receiver's code again.")).await;
        bail!("invalid ticket secret");
    }
    if version != PROTOCOL_VERSION {
        let _ = write_msg(&mut stream, &reject("Update Proxima on both devices.")).await;
        bail!("unsupported protocol version {version}");
    }
    let sender_name = truncate_utf8(device_name.trim(), MAX_NAME_LEN).to_string();
    Ok(Authed { stream, sender, sender_name, session })
}

fn reject(reason: &str) -> Control {
    Control::Reject { reason: reason.to_string() }
}

async fn serve(
    shared: Arc<Shared>,
    authed: Authed,
    session: SessionId,
    mut data_rx: mpsc::Receiver<TlsStream>,
) -> Result<Outcome> {
    let Authed { stream, sender, sender_name, .. } = authed;
    let (mut recv, mut send) = tokio::io::split(stream);
    let welcome = Control::Welcome { device_name: shared.options.device_name.clone(), session };
    write_msg(&mut send, &welcome).await?;
    let offer = match read_msg(&mut recv).await? {
        Control::Offer(offer) => offer,
        other => bail!("expected an offer, got {other:?}"),
    };
    let id = transfer_id(&sender, &offer);

    let mut slot = shared.transfer.lock().await;
    let state = match slot.as_ref() {
        Some(state) if state.id == id => state.clone(),
        Some(_) => {
            let _ = write_msg(&mut send, &reject("This device is busy with another transfer.")).await;
            return Ok(Outcome::Busy);
        }
        None => {
            let files = match plan_files(&offer) {
                Ok(files) => files,
                Err(e) => {
                    let _ = write_msg(&mut send, &reject(&format!("{e:#}"))).await;
                    return Err(e);
                }
            };
            let incoming = IncomingOffer {
                sender_name,
                files: offer.files.len() as u64,
                bytes: offer.total_bytes(),
                sample: offer.files.iter().take(5).map(|f| f.path.clone()).collect(),
            };
            if !(shared.approve)(incoming).await {
                let _ = write_msg(&mut send, &reject("The receiver declined.")).await;
                return Ok(Outcome::Declined);
            }
            let (options, stats, chunk_size, id) =
                (shared.options.clone(), shared.stats.clone(), offer.chunk_size, id.clone());
            let opened =
                tokio::task::spawn_blocking(move || TransferState::open(id, chunk_size, files, &options, stats))
                    .await?;
            let state = match opened {
                Ok(state) => state,
                Err(e) => {
                    let _ = write_msg(&mut send, &reject(&format!("{e:#}"))).await;
                    return Err(e);
                }
            };
            spawn_checkpointer(Arc::downgrade(&state));
            *slot = Some(state.clone());
            state
        }
    };
    drop(slot);

    write_msg(&mut send, &Control::Accept { have: state.have_list() }).await?;

    // The sender pings every half second. Silence means the link is gone, even if
    // TCP hasn't noticed yet.
    let (lost_tx, mut lost_rx) = mpsc::unbounded_channel();
    let _reader = AbortOnDrop(tokio::spawn(async move {
        loop {
            match tokio::time::timeout(HEARTBEAT_TIMEOUT, read_msg(&mut recv)).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => break lost_tx.send(e),
                Err(_) => {
                    break lost_tx.send(anyhow!("no word from the sender for {} seconds", HEARTBEAT_TIMEOUT.as_secs()))
                }
            }
        }
    }));

    let (resend_tx, mut resend_rx) = mpsc::unbounded_channel();
    let mut readers = JoinSet::new();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut complete = state.complete.subscribe();
    loop {
        tokio::select! {
            done = async { complete.wait_for(|done| *done).await.map(|_| ()) } => {
                done?;
                break;
            }
            Some(stream) = data_rx.recv() => {
                readers.spawn(read_data(stream, state.clone(), resend_tx.clone()));
            }
            Some(chunks) = resend_rx.recv() => {
                tokio::time::timeout(HEARTBEAT_TIMEOUT, write_msg(&mut send, &Control::Resend { chunks })).await??;
            }
            Some(e) = lost_rx.recv() => return Err(e),
            Some(joined) = readers.join_next() => joined??,
            _ = heartbeat.tick() => {
                let p = shared.stats.snapshot();
                let progress = Control::Progress { files: p.done_files, bytes: p.done_bytes };
                tokio::time::timeout(HEARTBEAT_TIMEOUT, write_msg(&mut send, &progress)).await??;
            }
        }
    }

    let summary = state.summary();
    write_msg(&mut send, &Control::Complete { files: summary.files, bytes: summary.bytes }).await?;
    // Give the sender a moment to read the result and close the connection itself.
    let _ = tokio::time::timeout(Duration::from_secs(2), lost_rx.recv()).await;
    state.cleanup();
    Ok(Outcome::Completed(summary))
}

async fn read_data(
    mut stream: TlsStream,
    state: Arc<TransferState>,
    resend: mpsc::UnboundedSender<Vec<ChunkRef>>,
) -> Result<()> {
    let mut header = [0u8; CHUNK_HEADER_LEN];
    let mut buf = Vec::new();
    while read_header(&mut stream, &mut header).await? {
        let h = ChunkHeader::decode(&header);
        let len = state.expected_len(&h)?;
        buf.resize(len as usize, 0);
        stream.read_exact(&mut buf).await?;
        let st = state.clone();
        let data = std::mem::take(&mut buf);
        let (data, stored) = tokio::task::spawn_blocking(move || {
            let stored = st.store_chunk(h, &data);
            (data, stored)
        })
        .await?;
        buf = data;
        let bad = stored?;
        if !bad.is_empty() {
            let _ = resend.send(bad);
        }
    }
    Ok(())
}

/// The receiver derives the ID itself, so a re-offer of the same files from the
/// same sender always maps to the same journal.
fn transfer_id(sender: &Fingerprint, offer: &Offer) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(sender);
    hasher.update(&serde_json::to_vec(offer).expect("offers always serialize"));
    hasher.finalize().to_hex().to_string()
}

struct RecvFile {
    rel: PathBuf,
    size: u64,
    chunks: u32,
    modified: Option<u64>,
}

fn plan_files(offer: &Offer) -> Result<Vec<RecvFile>> {
    ensure!(
        (MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&offer.chunk_size),
        "unsupported chunk size {}",
        offer.chunk_size
    );
    ensure!(!offer.files.is_empty() && offer.files.len() <= MAX_FILES, "invalid number of files");
    let mut cleaned = Vec::with_capacity(offer.files.len());
    for f in &offer.files {
        ensure!(f.size / (offer.chunk_size as u64) < u32::MAX as u64, "{} is too large", f.path);
        cleaned.push(safety::clean_relative_path(&f.path)?);
    }
    let paths = safety::unique_paths(cleaned)?;
    Ok(offer
        .files
        .iter()
        .zip(paths)
        .map(|(f, rel)| RecvFile {
            rel,
            size: f.size,
            chunks: chunk_count(f.size, offer.chunk_size),
            modified: f.modified,
        })
        .collect())
}

struct Slot {
    have: Vec<u8>,
    hashes: Vec<[u8; 32]>,
    remaining: u32,
    handle: Option<Arc<File>>,
    done: bool,
}

/// Everything known about one incoming transfer. Shared by every connection the
/// sender makes, so reconnects pick up exactly where the last one stopped.
struct TransferState {
    id: String,
    root: PathBuf,
    state_dir: PathBuf,
    chunk_size: u32,
    verify_on_disk: bool,
    files: Vec<RecvFile>,
    slots: Vec<Mutex<Slot>>,
    journal: Mutex<Journal>,
    /// Part files written since the last checkpoint.
    dirty: Mutex<HashMap<u32, Arc<File>>>,
    remaining_files: AtomicU64,
    complete: watch::Sender<bool>,
    stats: Arc<Stats>,
    started: Instant,
}

impl TransferState {
    fn open(
        id: String,
        chunk_size: u32,
        files: Vec<RecvFile>,
        options: &ReceiveOptions,
        stats: Arc<Stats>,
    ) -> Result<Arc<Self>> {
        let root = options.dir.clone();
        let state_dir = root.join(STATE_DIR).join(&id[..32]);
        fs::create_dir_all(&state_dir)?;
        let (journal, replay) = Journal::open(&state_dir.join("journal"))?;
        let slots = files
            .iter()
            .map(|f| {
                Mutex::new(Slot {
                    have: vec![0; bitmap_len(f.chunks)],
                    hashes: vec![[0; 32]; f.chunks as usize],
                    remaining: f.chunks,
                    handle: None,
                    done: false,
                })
            })
            .collect();
        let file_count = files.len() as u64;
        let state = Self {
            id,
            root,
            state_dir,
            chunk_size,
            verify_on_disk: options.verify_on_disk,
            files,
            slots,
            journal: Mutex::new(journal),
            dirty: Mutex::new(HashMap::new()),
            remaining_files: AtomicU64::new(file_count),
            complete: watch::channel(false).0,
            stats,
            started: Instant::now(),
        };
        let with_parts = state.replay(replay);
        state.check_space(&with_parts)?;
        for i in 0..state.files.len() as u32 {
            let ready = {
                let slot = state.slot(i);
                !slot.done && slot.remaining == 0
            };
            if ready {
                state.finish_file(i)?;
            }
        }
        state.publish_progress();
        Ok(Arc::new(state))
    }

    /// Applies the journal. Returns the files that already have a part file.
    fn replay(&self, replay: Replay) -> HashSet<u32> {
        let count = self.files.len();
        let mut finished = HashSet::new();
        for (i, name) in replay.done {
            if i as usize >= count {
                continue;
            }
            finished.insert(i);
            let f = &self.files[i as usize];
            let on_disk = safety::clean_relative_path(&name)
                .ok()
                .map(|parts| self.root.join(parts.iter().collect::<PathBuf>()))
                .and_then(|path| fs::metadata(path).ok())
                .is_some_and(|meta| meta.len() == f.size);
            if on_disk {
                self.mark_done(i);
            }
        }

        let mut with_parts = HashSet::new();
        let mut part_exists = HashMap::new();
        let mut unverified: HashMap<u32, Vec<(u32, [u8; 32])>> = HashMap::new();
        let mut without_part: HashMap<u32, HashMap<u32, [u8; 32]>> = HashMap::new();
        for rec in replay.chunks {
            let i = rec.file;
            if i as usize >= count || rec.chunk >= self.files[i as usize].chunks || finished.contains(&i) {
                continue;
            }
            if !*part_exists.entry(i).or_insert_with(|| self.part_path(i).exists()) {
                without_part.entry(i).or_default().insert(rec.chunk, rec.hash);
                continue;
            }
            with_parts.insert(i);
            if rec.trusted {
                self.mark(i, rec.chunk, rec.hash);
            } else {
                unverified.entry(i).or_default().push((rec.chunk, rec.hash));
            }
        }

        // Chunks after the last checkpoint may not have reached the disk before a
        // crash, so only count the ones whose bytes still match.
        for (i, chunks) in unverified {
            let Ok(file) = File::open(self.part_path(i)) else { continue };
            let size = self.files[i as usize].size;
            let mut buf = Vec::new();
            for (c, hash) in chunks {
                let (offset, len) = chunk_range(size, self.chunk_size, c);
                buf.resize(len as usize, 0);
                if disk::read_exact_at(&file, &mut buf, offset).is_ok()
                    && blake3::hash(&buf) == blake3::Hash::from(hash)
                {
                    self.mark(i, c, hash);
                }
            }
        }

        // A file whose done record was lost in a crash is already under its final
        // name. Recognize it by its chunk hashes instead of receiving it again.
        for (i, chunks) in without_part {
            let f = &self.files[i as usize];
            if chunks.len() != f.chunks as usize {
                continue;
            }
            let Ok(file) = File::open(self.root.join(&f.rel)) else { continue };
            if file.metadata().map(|m| m.len()).ok() != Some(f.size) {
                continue;
            }
            let hashes: Vec<[u8; 32]> = (0..f.chunks).map(|c| chunks[&c]).collect();
            if matches!(verify_part(&file, f.size, self.chunk_size, &hashes), Ok(bad) if bad.is_empty()) {
                self.mark_done(i);
                let _ = self.journal.lock().unwrap().file_done(i, &rel_string(&f.rel));
            }
        }
        with_parts
    }

    fn mark_done(&self, i: u32) {
        let chunks = self.files[i as usize].chunks;
        let mut slot = self.slot(i);
        slot.done = true;
        slot.remaining = 0;
        slot.hashes = Vec::new();
        for c in 0..chunks {
            bit_set(&mut slot.have, c);
        }
        drop(slot);
        self.remaining_files.fetch_sub(1, Ordering::Relaxed);
    }

    fn check_space(&self, with_parts: &HashSet<u32>) -> Result<()> {
        let needed = (0..self.files.len() as u32)
            .filter(|i| !with_parts.contains(i) && !self.slot(*i).done)
            .fold(0u64, |sum, i| sum.saturating_add(self.files[i as usize].size));
        let free = disk::available_space(&self.root).unwrap_or(u64::MAX);
        if needed > free {
            return Err(Fatal(format!(
                "Not enough free space: this transfer needs {}, but only {} is free.",
                human_bytes(needed),
                human_bytes(free)
            ))
            .into());
        }
        Ok(())
    }

    fn expected_len(&self, h: &ChunkHeader) -> Result<u32> {
        let f = self.files.get(h.file as usize).context("chunk for an unknown file")?;
        ensure!(h.chunk < f.chunks, "chunk index out of range");
        let (_, len) = chunk_range(f.size, self.chunk_size, h.chunk);
        ensure!(h.len == len, "chunk has the wrong length");
        Ok(len)
    }

    /// Verifies and writes one chunk. Returns chunks that must be sent again.
    fn store_chunk(&self, h: ChunkHeader, data: &[u8]) -> Result<Vec<ChunkRef>> {
        ensure!(blake3::hash(data) == blake3::Hash::from(h.hash), "a chunk failed verification");
        let file = {
            let mut slot = self.slot(h.file);
            if slot.done || bit_get(&slot.have, h.chunk) {
                return Ok(Vec::new());
            }
            self.handle(h.file, &mut slot)?
        };
        let (offset, _) = chunk_range(self.files[h.file as usize].size, self.chunk_size, h.chunk);
        disk::write_all_at(&file, data, offset)?;
        self.dirty.lock().unwrap().insert(h.file, file);
        self.journal.lock().unwrap().chunk(h.file, h.chunk, &h.hash)?;
        let finished = {
            let mut slot = self.slot(h.file);
            if slot.done || bit_get(&slot.have, h.chunk) {
                return Ok(Vec::new());
            }
            bit_set(&mut slot.have, h.chunk);
            slot.hashes[h.chunk as usize] = h.hash;
            slot.remaining -= 1;
            slot.remaining == 0
        };
        self.stats.add_bytes(data.len() as u64);
        if finished {
            self.finish_file(h.file)
        } else {
            Ok(Vec::new())
        }
    }

    /// Flushes, optionally re-verifies from disk, restores the timestamp and moves
    /// the part file to its final name.
    fn finish_file(&self, i: u32) -> Result<Vec<ChunkRef>> {
        let f = &self.files[i as usize];
        let (file, hashes) = {
            let mut slot = self.slot(i);
            let file = self.handle(i, &mut slot)?;
            slot.handle = None;
            let hashes = if self.verify_on_disk { slot.hashes.clone() } else { Vec::new() };
            (file, hashes)
        };
        self.dirty.lock().unwrap().remove(&i);
        file.sync_all()?;
        if self.verify_on_disk {
            let bad = verify_part(&file, f.size, self.chunk_size, &hashes)?;
            if !bad.is_empty() {
                let mut slot = self.slot(i);
                for &c in &bad {
                    if bit_get(&slot.have, c) {
                        bit_clear(&mut slot.have, c);
                        slot.remaining += 1;
                        self.stats.sub_bytes(chunk_range(f.size, self.chunk_size, c).1 as u64);
                    }
                }
                return Ok(bad.into_iter().map(|chunk| ChunkRef { file: i, chunk }).collect());
            }
        }
        if let Some(ms) = f.modified {
            let _ = file.set_modified(UNIX_EPOCH + Duration::from_millis(ms));
        }
        drop(file);
        let name = self.place(i)?;
        self.journal.lock().unwrap().file_done(i, &name)?;
        {
            let mut slot = self.slot(i);
            slot.done = true;
            slot.hashes = Vec::new();
        }
        self.stats.add_file();
        if self.remaining_files.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.complete.send_replace(true);
        }
        Ok(Vec::new())
    }

    /// Never overwrites an existing file: `photo.jpg` becomes `photo (1).jpg`.
    fn place(&self, i: u32) -> Result<String> {
        let rel = &self.files[i as usize].rel;
        let name = rel.file_name().and_then(|n| n.to_str()).context("invalid file name")?;
        let mut target = rel.clone();
        let mut n = 1;
        while self.root.join(&target).exists() {
            target = rel.with_file_name(safety::numbered(name, n));
            n += 1;
        }
        fs::rename(self.part_path(i), self.root.join(&target))
            .with_context(|| format!("can't save {}", target.display()))?;
        Ok(rel_string(&target))
    }

    fn handle(&self, i: u32, slot: &mut Slot) -> Result<Arc<File>> {
        if let Some(file) = &slot.handle {
            return Ok(file.clone());
        }
        let path = self.part_path(i);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("can't create {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("can't write {}", path.display()))?;
        let size = self.files[i as usize].size;
        if file.metadata()?.len() != size {
            file.set_len(size)?;
        }
        let file = Arc::new(file);
        slot.handle = Some(file.clone());
        Ok(file)
    }

    fn part_path(&self, i: u32) -> PathBuf {
        let path = self.root.join(&self.files[i as usize].rel);
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(PART_SUFFIX);
        path.with_file_name(name)
    }

    fn mark(&self, i: u32, c: u32, hash: [u8; 32]) {
        let mut slot = self.slot(i);
        if !bit_get(&slot.have, c) {
            bit_set(&mut slot.have, c);
            slot.hashes[c as usize] = hash;
            slot.remaining -= 1;
        }
    }

    fn have_list(&self) -> Vec<FileHave> {
        self.slots
            .iter()
            .map(|slot| {
                let slot = slot.lock().unwrap();
                if slot.done {
                    FileHave::Complete
                } else if slot.have.iter().all(|b| *b == 0) {
                    FileHave::None
                } else {
                    FileHave::Partial { bitmap: slot.have.clone() }
                }
            })
            .collect()
    }

    fn publish_progress(&self) {
        let (mut files, mut bytes) = (0u64, 0u64);
        for (i, f) in self.files.iter().enumerate() {
            let slot = self.slots[i].lock().unwrap();
            if slot.done {
                files += 1;
                bytes += f.size;
                continue;
            }
            for c in 0..f.chunks {
                if bit_get(&slot.have, c) {
                    bytes += chunk_range(f.size, self.chunk_size, c).1 as u64;
                }
            }
        }
        self.stats.set_totals(self.files.len() as u64, self.total_bytes());
        self.stats.set_done(files, bytes);
    }

    fn total_bytes(&self) -> u64 {
        self.files.iter().fold(0u64, |sum, f| sum.saturating_add(f.size))
    }

    fn summary(&self) -> Summary {
        Summary { files: self.files.len() as u64, bytes: self.total_bytes(), elapsed: self.started.elapsed() }
    }

    /// Flushes the part files written since the last checkpoint, then marks the
    /// chunk records written before the snapshot as durable. Writers keep going
    /// during the flush: every chunk record counted in `covered` had its data
    /// written and its file marked dirty before the snapshot was taken.
    fn checkpoint(&self) -> io::Result<()> {
        let (dirty, covered) = {
            let journal = self.journal.lock().unwrap();
            let dirty = std::mem::take(&mut *self.dirty.lock().unwrap());
            (dirty, journal.chunk_records())
        };
        if dirty.is_empty() {
            return Ok(());
        }
        for file in dirty.values() {
            file.sync_data()?;
        }
        self.journal.lock().unwrap().checkpoint(covered)
    }

    fn cleanup(&self) {
        self.journal.lock().unwrap().close();
        let _ = fs::remove_dir_all(&self.state_dir);
        let _ = fs::remove_dir(self.root.join(STATE_DIR));
    }

    fn slot(&self, i: u32) -> MutexGuard<'_, Slot> {
        self.slots[i as usize].lock().unwrap()
    }
}

fn rel_string(path: &std::path::Path) -> String {
    path.iter().map(|c| c.to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Re-reads a file and returns the chunks whose bytes don't match their hashes.
/// Large files are checked on several threads.
fn verify_part(file: &File, size: u64, chunk_size: u32, hashes: &[[u8; 32]]) -> io::Result<Vec<u32>> {
    const CHUNKS_PER_THREAD: usize = 16;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let per_thread = hashes.len().div_ceil(threads).max(CHUNKS_PER_THREAD);
    let check = |first: usize, group: &[[u8; 32]]| -> io::Result<Vec<u32>> {
        let mut bad = Vec::new();
        let mut buf = Vec::new();
        for (i, expected) in group.iter().enumerate() {
            let c = (first + i) as u32;
            let (offset, len) = chunk_range(size, chunk_size, c);
            buf.resize(len as usize, 0);
            disk::read_exact_at(file, &mut buf, offset)?;
            if blake3::hash(&buf) != blake3::Hash::from(*expected) {
                bad.push(c);
            }
        }
        Ok(bad)
    };
    if hashes.len() <= per_thread {
        return check(0, hashes);
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = hashes
            .chunks(per_thread)
            .enumerate()
            .map(|(n, group)| scope.spawn(move || check(n * per_thread, group)))
            .collect();
        let mut bad = Vec::new();
        for handle in handles {
            bad.extend(handle.join().expect("verify thread panicked")?);
        }
        Ok(bad)
    })
}

fn spawn_checkpointer(weak: Weak<TransferState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CHECKPOINT_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            let Some(state) = weak.upgrade() else { break };
            if *state.complete.borrow() {
                break;
            }
            let _ = tokio::task::spawn_blocking(move || state.checkpoint()).await;
        }
    });
}
