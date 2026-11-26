# Testing

## On this PC

```powershell
. .\tools\env.ps1
cargo test --workspace            # engine, including a full transfer over a simulated USB cable
cd app; flutter test              # flows on a fake engine, plus golden screenshots
```

Golden screenshots are rendered on Windows, so CI on Linux skips them
(`--exclude-tags golden`). After an intended UI change, refresh them with
`flutter test --update-goldens` and look at the images in `app/test/goldens`.

## On two phones

The radios, the USB cable and the updater only run on real hardware. Use two
Android 11+ phones, called **A** (sends) and **B** (receives). Ideally one has
NFC, and you have a USB-C cable that carries data.

Install the same build on both: the APK from a GitHub release, or a local build
with `adb install`:

```powershell
cd app; flutter build apk --release --split-per-abi
adb -s <serial> install -r build\app\outputs\flutter-apk\app-arm64-v8a-release.apk
```

To check that files arrived intact, compare hashes on both phones:

```powershell
adb -s <A> shell sha256sum "/sdcard/<folder>/<file>"
adb -s <B> shell sha256sum "/sdcard/Download/Nearvolt/<file>"
```

### Permissions and switches

Start with a fresh install, Bluetooth off.

| # | Steps | Expected |
|---|---|---|
| P1 | Open the app. | Home with Send and Receive; no permission prompt yet. |
| P2 | Tap Send. | Android asks for Nearby devices and notifications, then asks to turn on Bluetooth. Allow all: no notice on the page. |
| P3 | Reinstall, tap Send and deny Nearby devices. | A notice asks to allow it. Tap Allow and grant it: the notice goes away and Bluetooth phones start appearing. |
| P4 | Deny Nearby devices twice. | The notice's button says Settings and opens Nearvolt's app settings; granting it there and coming back clears the notice. |
| P5 | Turn Wi-Fi off, open Send. | A Wi-Fi notice; Turn on opens Android's Wi-Fi panel. |
| P6 | Android 11 or 12 only: turn Location off, open Send. | A Location notice; Turn on opens Location settings. |
| P7 | With Send open, turn Bluetooth off from quick settings. | The Bluetooth notice appears without leaving the page. |

### Ways to connect

| # | Steps | Expected |
|---|---|---|
| 1 | Both on the same Wi-Fi. B opens Receive, A opens Send. | B appears in A's Nearby list within 3 s, marked "Same Wi-Fi". |
| 2 | On A, tap B, choose files, tap Send. | Both screens show the same 4-digit code. B accepts; the transfer runs and ends with "All N files verified". Hashes match. |
| 3 | Both with Wi-Fi on but not connected to the same network (or none). B opens Receive. | B appears on A marked "Bluetooth". Sending joins B's Wi-Fi Direct group with no pop-up and transfers as in 2. |
| 4 | B opens Receive. On A, choose files, tap Scan code, scan B's QR code. | Sending starts at once with no code; B is asked to accept. |
| 5 | B opens Receive. On A, choose files, then hold the backs of the phones together. | Sending starts at once with no code. |
| 6 | B opens Receive, A opens Send, then connect the phones with a USB cable. Answer Android's prompts ("Allow access", "Open Nearvolt"). | A shows "USB cable" (or B's model) in Nearby. Sending shows a code on both, as in 2. |
| 7 | B closes Receive. | B disappears from A's Nearby list. |
| 8 | B declines an offer. | A shows "Transfer stopped". |

### Keeping the connection

Start a transfer of at least 5 GB for each of these.

| # | Steps | Expected |
|---|---|---|
| 9 | Turn Wi-Fi off on B for 10 seconds, then on. | A shows "Reconnecting…", then continues from where it stopped and completes. Hashes match. |
| 10 | Pull the USB cable for 10 seconds, then plug it back in. | As in 9. |
| 11 | Lock both screens until the transfer ends. | It keeps running, with progress in the notification. |
| 12 | Force-stop the app on B mid-transfer (`adb shell am force-stop app.nearvolt`). Reopen Receive on B, then on A tap Send again (the files stay chosen). | The transfer resumes near where it stopped instead of starting over, and completes with matching hashes. |
| 13 | Force-stop A mid-transfer. Reopen it on A, choose the same files and send again. | As in 12. |

### Big and many

| # | Steps | Expected |
|---|---|---|
| 14 | Send as much as B's storage allows (aim for 100 GB or more), mixing large videos and thousands of photos. | Completes with "All N files verified". Spot-check hashes. Record the speed below. |
| 15 | Send a folder with nested folders and a few thousand small files. | The same folder structure appears under `Download/Nearvolt`, and the photos show up in the gallery. |

### Updates

| # | Steps | Expected |
|---|---|---|
| 16 | Install the latest release, open the app and tap Check for updates. | "Nearvolt N is the latest version." Nothing is checked until the button is tapped. |
| 17 | Push a change to `main` and wait for its release. Tap Check for updates. | It downloads version N+1. The first time, Android opens "Install unknown apps"; turn it on and go back, and the update continues by itself. Nearvolt closes for a moment; reopened, it runs N+1. |
| 18 | Repeat 17 on Android 12 or later. | No prompt at all this time. |
| 19 | On B, tap Check for updates and, while it downloads, open Receive and accept a transfer from A. | Nothing is installed while the transfer runs; the update card says to finish the transfer first. |

### Speeds

| Link | Phones | Data | MB/s |
|---|---|---|---|
| Same Wi-Fi | | | |
| Wi-Fi Direct | | | |
| USB | | | |
