# Performance map

How fast Nearvolt is today, what limits it for each way of connecting, and the order of
work to make it faster. Each number is marked **measured** (on the two test phones, a
Samsung SM-S938B and an iQOO I2212, or on a PC) or **estimated** (from published link
rates). Nothing estimated is a promise.

An interactive version of the charts is in the Cursor canvas `nearvolt-performance-map`.

## Where we are

| What | Value | Basis |
|---|---|---|
| Wi-Fi Direct, one 315 MB file | 39.4 and 43.4 MB/s | measured |
| Over the router (5 GHz, strong signal) | 32.7 MB/s | measured |
| Over the router, 2.4 GHz (what the router is on now) | about 5 to 8 MB/s | estimated |
| Engine on a PC over loopback, one large file | about 270 MB/s | measured |
| Engine on a PC, 2,000 small files (400 MB) | 45 to 126 MB/s | measured |
| Samsung storage | 1.7 GB/s write (with fsync), 4.9 GB/s read | measured |
| Tap to connected (NFC) | 1.1 s | measured |

**Finding.** The Wi-Fi Direct link reports 780 to 866 Mbps, about 55 to 65 MB/s of real
data. We get 43, so roughly 70 to 80% of what the link can carry. Storage is more than
40 times faster than the transfer. The next gains come from a wider, cleaner Wi-Fi Direct
channel, not from faster code.

**How long 1 GB takes**

| Speed | Source | 1 GB |
|---|---|---|
| 43 MB/s | today, Wi-Fi Direct (measured) | about 24 s |
| 60 MB/s | ceiling of today's 80 MHz link (estimated) | about 17 s |
| 150 MB/s | Wi-Fi 6E pair, 160 MHz (estimated) | about 7 s |
| 400 MB/s | Wi-Fi 7 pair, 320 MHz (estimated) | about 2.6 s |

Gigabytes "in a moment" needs a faster link than today's: 330 MB/s for 3 seconds per GB.
The Samsung is Wi-Fi 7, but the iQOO is Wi-Fi 6E and sets the ceiling for that pair.

## The path of a file

```mermaid
flowchart LR
    Disk1[Read file] --> Hash1[Hash BLAKE3]
    Hash1 --> Enc[Encrypt TLS 1.3 AES-GCM]
    Enc --> Link{{Wi-Fi Direct, router or USB}}
    Link --> Dec[Decrypt]
    Dec --> Write[Write part file]
    Write --> Verify[Re-read and hash]
    Verify --> Rename[Rename into place]
```

| Stage | Cost on these phones | Basis | Verdict |
|---|---|---|---|
| Read on sender | about 4.9 GB/s cached | measured | not a limit |
| Hash | several GB/s per big core | estimated | not a limit |
| Encrypt | 1 GB/s or more per core with hardware AES | estimated | not a limit unless on a little core |
| TCP and Wi-Fi link | 55 to 65 MB/s possible, 43 achieved | measured | the limit today |
| Write on receiver | 1.7 GB/s (Samsung), 489 MB/s (iQOO) | measured | not a limit for one file |
| Verify | runs after the last byte, about 1 to 2 s per GB | estimated | adds to total time |
| Per-file overhead | 2,000 files: 45 to 126 MB/s against 270 | measured | the limit for many small files |

## Every way to connect

| Way | How it starts | Approval | Setup | Speed | Limited by | Best next step |
|---|---|---|---|---|---|---|
| NFC tap | 197-byte code read in 78 ms, data over Wi-Fi Direct | none | 1.1 s (measured) | 39 to 43 MB/s | Wi-Fi Direct channel and width | choose the group channel and width, stripe one big file over several streams |
| Scan code | camera reads the same code | none | 0.6 to 1.5 s join + 0.1 s | 39 to 43 MB/s | same as NFC | same as NFC |
| Nearby | **Removed on 28 Nov 2026**: phones are no longer listed over the network or Bluetooth. Tap, scan and USB remain | n/a | n/a | n/a | n/a | n/a |
| USB | accessory link, no radio | receiver accepts once | not measured | not measured | cable, port, accessory driver | measure; accessory mode may run at USB 2.0 speed (about 38 MB/s) |

### NFC, in detail

NFC carries only the code. Data goes over the Wi-Fi Direct group the receiver made when
Receive opened. A tap on the iQOO, measured:

| Step | Time |
|---|---|
| Touch to code read | 78 ms |
| Start the connection | 130 ms |
| Join the Wi-Fi Direct group | 845 ms (other joins: 620 and 870 ms) |
| Secure link and ready | 67 ms |
| Total | 1.12 s |

Order of work: (1) choose the group channel and width on purpose and measure speed per
channel; (2) send one big file over several streams; (3) cut the join time; (4) make a
failed tap say why.

## Techniques

| Id | Technique | Helps | Expected gain | Effort |
|---|---|---|---|---|
| L1 | Choose the Wi-Fi Direct channel: a non-radar 80 MHz block, or the router's own channel when allowed | tap, scan | up to 1.3 to 2x if the group shares airtime with the router today | low |
| L2 | Ask for the widest group: 160 MHz, and 6 GHz where both phones support it | tap, scan | 2 to 4x (estimated) | medium |
| L3 | ~~Move Nearby to Wi-Fi Direct~~ not needed: Nearby was removed | none | none | none |
| T1 | Stripe one big file over several streams | all | 1.1 to 1.5x if one stream cannot fill the link | medium |
| T2 | Larger socket buffers, adaptive stream count | all | small | low |
| C1 | Run engine threads on the big cores with performance hints | all | unknown; only if a phone is CPU-bound | low |
| F1 | Sync once per batch, not once per file | many small files | 2 to 4x | low |
| F2 | Pack small files into one stream with a manifest | many small files | 3 to 5x (estimated) | high |
| F3 | Hash while receiving instead of re-reading | all | saves the verify pass | medium |
| U1 | Measure USB 3.x accessory speed before promising it | USB | unknown | low |
| X1 | Skip files already on the other phone | copy phone | large on a second run | done (branch) |

### Tried and rejected

Overlapping disk, hash and network with a read-ahead queue on the sender and a
write-behind queue on the receiver. Randomised A/B runs on a PC (10 runs each, 200 MB):
current code median 270 MB/s, overlapped variants 90 to 143 MB/s. The gain could still
exist on a phone, but without a phone measurement it was not shipped.

## The research, and what fits Nearvolt

The write-up this section answers says one thing first: hardware sets the speed, and
software's job is to use all of it. That holds here. Nearvolt moves files between Android
phones, over Wi-Fi Direct, a router, or a USB cable, so the "fastest link" is a phone's
radio or cable, not a data-centre card. Its techniques, checked one by one:

| Technique | Verdict for Nearvolt | Why |
|---|---|---|
| Use the fastest physical link | **Use** (L1, L2, U1) | Biggest lever: a wider Wi-Fi Direct channel, and USB only if measured faster |
| Many streams in parallel, chunks of 4 to 64 MB | **Use** (T1) | 8 streams are open today, but one file uses one of them, and the receiver only accepts a range that runs to the end of the file |
| Adaptive concurrency from measured throughput | **Use** | No fixed stream count fits a 2.4 GHz router and a 6 GHz group alike |
| Skip data the other side has (hash chunks) | **Use** | Already skips files by size on the copy branch; chunk hashes extend it to partial files and re-runs |
| Resume by finished chunk | **Use** | Resume is by file tail today; a chunk list loses at most one chunk |
| Check each chunk when it arrives | **Use** (F3) | Today's check re-reads the whole file after the last byte |
| Big sequential reads and writes | Have it (1 MiB); try 4 MiB | Storage is not the limit on these phones |
| Compression, only when it helps | **Use sparingly** | Photos and videos do not compress. Worth it only for compressible files on a slow link (a 2.4 GHz router, USB 2.0); sample the first block with LZ4 first |
| Authenticated encryption | Have it | TLS 1.3 with hardware AES; far faster than the link, so not a cost |
| Zero-copy (`sendfile`, `splice`, `MSG_ZEROCOPY`) | **Does not apply** | The data is encrypted in the app, so it must pass through memory anyway. The copy it would save is worth GB/s against a link of 43 MB/s |
| `io_uring` | **Does not apply** | Recent Android versions block it for apps (check on the target phones), and I/O is not the limit |
| QUIC | Not now | Wi-Fi Direct and a cable lose few packets, and UDP in the app costs CPU. Revisit if retransmits show up in measurements |
| RDMA | **Does not apply** | Needs special network cards |
| Buffer pools, read-ahead queues | Tried, no gain | See "Tried and rejected"; measure on a phone before trying again |

### Combined: one engine, a recipe per connection

The common core (engine v2):

1. **Chunk protocol.** Files are split into chunks (8 MiB to start). The receiver accepts
   any chunk at any offset and writes it in place. This one change allows striping one
   big file over several streams, chunk-level resume and chunk-level skip.
2. **Per-chunk hash**, checked in memory when the chunk arrives. The disk read-back becomes
   an option, not a second full pass.
3. **Adaptive controller.** Every second or two, compare MB/s with and without one more
   stream (and a different chunk size); keep what helps; back off when the CPU or memory
   is the limit or retransmits rise.
4. **Link chooser.** Pick the best link available for the pair and tell the person when a
   link will be slow.

| Connection | Link and channel | Streams and chunks | Compression | Also |
|---|---|---|---|---|
| NFC tap, scan code | Wi-Fi Direct, with a chosen non-radar 80 MHz channel, 160 MHz or 6 GHz when both phones support it | striped over 4 to 16 streams, adaptive | off (fast link) | group made ahead when Receive opens; high-performance Wi-Fi lock |
| USB cable | accessory link; measure the speed first. If it runs at USB 2.0 speed, say so and prefer Wi-Fi Direct | streams share one pipe, so fewer, bigger chunks; raise the multiplexer window | on for compressible files while the pipe is USB 2.0 | decide what to offer only after measuring |
| Copy old phone | the best of the above | chunk-hash skip on re-runs; bundle small files into one stream | off | one sync per batch, not per file |

### How we will know it is "super best"

| Goal | Target |
|---|---|
| Use what the link allows | at least 90% of the measured ceiling of that link (today about 72% on Wi-Fi Direct) |
| Setup | tap to ready in 1 s or less (today 1.1 s) |
| Many small files | at least 60% of large-file speed (today 17 to 47%) |
| Resume | a break costs at most one chunk |
| CPU | one big core under 60% while sending |

The honest ceiling: terabytes per second does not exist between phones. For this pair the
best realistic speed is about 150 MB/s on Wi-Fi 6E, and a Wi-Fi 7 pair or USB 3.x is
needed to go beyond that.

## Plan, one connection type at a time

| Phase | Work | Needs | Proof |
|---|---|---|---|
| 0 | Measure on the devices: CPU per thread, link rate and width, time per stage in the log | both phones on adb | whether either phone is CPU-bound |
| 1 | Tap and scan: choose the channel and width, then measure each | phase 0 | speed per channel, repeated runs |
| 2 | **Done (v5 protocol):** chunk protocol: a big file is striped over all streams in 8 MiB chunks, each chunk checked as it arrives, resume sends only the gaps. Still to do: hash-based skip, per-batch sync | phase 1 | on this PC's loopback one 1.6 GB file: 1 stream about 500 MB/s, 8 streams about 900 MB/s; a break loses only the chunks in flight |
| 3 | Both phones join the receiver's own Wi-Fi Direct network as a normal Wi-Fi client, so Android drops the router while the transfer runs | phase 1 | same file with and without the router connection |
| 4 | USB: measure, then decide how to use it | a cable | speed at USB 2.0 and 3.x ports |
| 5 | Many small files: bundle stream | phase 2 | files per second for 2,000 files |
| 6 | Adaptive tuning and a built-in speed test screen | all above | the same numbers without a laptop |

## Still to measure

- CPU per thread during a transfer on both phones.
- The real channel width and link rate of the Wi-Fi Direct group.
- USB speed at a USB 2.0 and a USB 3.x port.
- Many small files on the phones themselves.

## How the numbers were taken

- Phone transfers: the `[speed]` and `batch completed` lines in `nearvolt.log`, one 315 MB
  file, two phones side by side.
- PC: `tools/bench` `local` mode, release build, files on the same SSD. The SSD slows
  after many gigabytes, so runs were alternated in random order and repeated.
- Storage: `dd` over adb on the Samsung. Wi-Fi capabilities: `dumpsys wifi`.
