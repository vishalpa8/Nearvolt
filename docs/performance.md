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
| Nearby | name service + Bluetooth find the phone in 0.2 s, data over the router | receiver accepts with a code | 2.5 s | 32.7 MB/s on 5 GHz; about 5 to 8 on 2.4 GHz (estimated) | the router | after approval, move data to Wi-Fi Direct over the encrypted link |
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
| L1 | Choose the Wi-Fi Direct channel: a non-radar 80 MHz block, or the router's own channel when allowed | tap, scan, Nearby after L3 | up to 1.3 to 2x if the group shares airtime with the router today | low |
| L2 | Ask for the widest group: 160 MHz, and 6 GHz where both phones support it | tap, scan, Nearby after L3 | 2 to 4x (estimated) | medium |
| L3 | Move Nearby to Wi-Fi Direct after approval, sending the group details over the encrypted link | Nearby | 1.3x on a good router, 5x or more on 2.4 GHz | medium |
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

## Plan, one connection type at a time

| Phase | Work | Needs | Proof |
|---|---|---|---|
| 0 | Measure on the devices: CPU per thread, link rate and width, time per stage in the log | both phones on adb | whether either phone is CPU-bound |
| 1 | Tap and scan: choose the channel and width, then measure each | phase 0 | speed per channel, repeated runs |
| 2 | Stripe a big file; per-batch sync | phase 1 | same file with 1, 4 and 8 streams |
| 3 | Nearby: upgrade to Wi-Fi Direct after approval | phase 1 | same file over the router against Wi-Fi Direct |
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
