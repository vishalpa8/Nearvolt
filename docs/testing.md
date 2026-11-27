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

## When something goes wrong

- **The log:** Files app › Downloads › Nearvolt › `nearvolt.log`. It records
  each start (app version, phone, Android version), every crash with its stack
  trace, what Android was missing when a permission or switch was asked for,
  each update step with Android's reason when an install fails, and each
  connection's milestones: connected, each batch with its size, time and speed,
  drops, reconnects and disconnect. It holds no file contents, no file names,
  no codes and no full IP addresses (`10.0.3.7` is written `10.0.x.x`; the
  Wi-Fi Direct range `192.168.49.x` stays readable because it shows how the
  phones linked). Phone names are not written either.
  It keeps the last 7 days and stays under 5 MB, so it never fills the phone.
  While the app is in development it also records what you did and what the app
  did, so any problem can be traced afterwards. Tags to look for:
  - `[tap]` the label of every button tapped, and `[screen]` every screen,
    dialog and sheet that opens or closes.
  - `[message]` every message the app shows, `[app]` and `[activity]` when the
    app goes to the background or comes back, `[print]` the app's own prints.
  - `[call]` every request the app makes to Android, with how it ended and how
    long it took (a request with no "answered" line never came back).
  - `[setup]` what each page is missing, `[event]` each step of a transfer
    (batch started, completed, failed), `[error]` anything that went wrong.
  - `[bluetooth]`, `[nfc]`, `[nearby]`, `[usb]`, `[wifi-direct]`: what each
    radio did. A failure is written, never swallowed.
  Phone names can appear in tap labels, so share the log only with people you
  trust. All of this is switched off by `detailedLogging` in `app/lib/debug.dart`
  before a public launch.
  Share it, or with USB debugging:
  `adb pull /sdcard/Download/Nearvolt/nearvolt.log`.
- **More than one log file:** Android ties a file in Downloads to the install
  that created it. After Nearvolt is uninstalled and installed again, it can't
  write or delete the old log, so it starts `nearvolt-2.log` and keeps to that.
  Delete old ones by hand in the Files app; logs named `nearvolt-log….txt` are
  from older versions and can all go.
- **The app won't open, so it can't update itself:** in the phone's browser open
  <https://github.com/vishalpa8/Nearvolt/releases/latest>, download the
  `arm64-v8a` APK and install it over the old one. If Android says "App not
  installed", uninstall Nearvolt first; received files stay in Downloads.
- **"Samsung's Auto Blocker stopped the update"** (in the log:
  `Self update is blocked by unknown source package`): Auto Blocker lets only
  Google Play and the Galaxy Store install apps, and newer Galaxy phones have it
  on from the start. Turn it off in Settings › Security and privacy › Auto
  Blocker, tap Check for updates again (the download is reused), and turn it
  back on afterwards if you like.

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
| P1 | Open the app, then tap Send. | Send lists Tap phones, Scan code, Nearby phones and USB cable; no permission prompt yet. |
| P2 | Tap Nearby phones. | Android asks for Nearby devices and notifications, then asks to turn on Bluetooth. Allow all: the page opens with no gate. |
| P3 | Reinstall, open Nearby phones and deny Nearby devices. | The page doesn't open; a "Turn these on to continue" screen asks to allow it. Tap Allow and grant it: the page opens and Bluetooth phones start appearing. |
| P4 | Deny Nearby devices twice. | The button says Settings and opens Nearvolt's app settings; granting it there and coming back opens the page. |
| P5 | Turn NFC off, open Tap phones. Then, on the other phone, turn NFC off and open Receive. | Neither page opens; each shows an NFC gate whose Turn on button opens Android's NFC settings, and the page opens once NFC is on. No permission or Bluetooth prompt on Tap. |
| P6 | Turn Wi-Fi off, open Nearby phones. | The page stays closed behind a Wi-Fi gate; Turn on opens Android's Wi-Fi panel. Also try Receive and Scan code: Receive doesn't start, and the camera doesn't open until Wi-Fi is on. |
| P7 | Android 11 or 12 only: turn Location off, open Nearby phones. | A Location gate; Turn on opens Location settings. |
| P8 | With Nearby phones open, turn Bluetooth off from quick settings. | The page closes behind the Bluetooth gate, and opens again when it is back on. |

### Ways to connect

| # | Steps | Expected |
|---|---|---|
| 1 | Both on the same Wi-Fi. B opens Receive, A opens Send › Nearby phones. | B appears within 3 s, marked "Same Wi-Fi". |
| 2 | On A, tap B. | Both screens show the same 4-digit code. B accepts; A shows "Connected". |
| 3 | On A, tap Add files and pick some. Before they finish, add more. | The first batch shows "Sending", the second "Waiting its turn", then each "Sent and verified". B lists the same as "Received and verified". Hashes match. |
| 4 | Both with Wi-Fi on but not connected to the same network (or none). B opens Receive. | B appears on A marked "Bluetooth". Connecting joins B's Wi-Fi Direct group with no pop-up; then as in 2 and 3. |
| 5 | B opens Receive. On A, Send › Scan code, scan B's QR code. | No code to compare; B is asked once to accept. |
| 6 | B opens Receive. On A, Send › Tap phones, then hold the backs together. | Connected at once, with no prompt on B. |
| 6a | As 6, but first choose files on A's Send page. | The files start sending as soon as A says Connected, with no "Add files" tap. |
| 6b | On A, choose a way to connect, then keep B's screen on its accept prompt for 10 seconds before accepting. | A shows "Connecting…" or "Waiting for B to accept" with Cancel and no Add files. B is asked once. After accepting, A says Connected and shows Add files and Disconnect. |
| 6c | Turn B's Wi-Fi off for 15 seconds while connected. | A shows "Link dropped · Reconnecting…" with Stop reconnecting and no Add files, then Connected again. |
| 7 | B opens Receive, A opens Send › USB cable, then connect the phones with a cable. Answer Android's prompts ("Allow access", "Open Nearvolt"). | A connects by itself; both show the same code, as in 2. |
| 8 | B closes Receive. | B disappears from A's Nearby list. |
| 9 | B declines. | A shows "Connection failed" with "The receiver declined." |
| 10 | While A and B are connected over the same Wi-Fi, look for B in a third phone's Nearby list and in its Wi-Fi Direct settings, and try connecting from the third phone with B's QR code. | B is in neither list, and the third phone is turned away: "This phone is connected to another device." |
| 11 | Tap Disconnect on B. | A shows "B disconnected" and a Done button. Both stop. |
| 11a | Send a large video and tap Disconnect on either phone halfway. | The receiver's `Download/Nearvolt` has no half file and no `.nvpart` file; files that had finished stay. Sending again starts the video from the beginning. |
| 11b | Fill the receiver's storage almost completely, then send a file bigger than the free space. | Both phones say, in words, that there isn't enough space (on the receiving phone's screen: "on this phone"). The sender doesn't sit on "Reconnecting…", and the connection stays so another batch can be sent. |
| 11c | With B's Receive prompt showing, have A cancel (Back). | The prompt closes by itself on B and says "A cancelled."; tapping nothing leaves B ready for the next phone. |
| 11d | With B's prompt showing for A, start a second phone C connecting to B. | C is told B is connected to another phone; B's prompt for A is unchanged, and Accept connects A, not C. |
| 11e | Run a port scanner, or open the receiver's port in a browser many times, while connected. | The transfer and Receive carry on; the log shows "N unknown connection attempts were ignored". |

### Keeping the connection

Send at least 5 GB for each of these.

| # | Steps | Expected |
|---|---|---|
| 12 | Turn Wi-Fi off on B for 10 seconds, then on. | A shows "Reconnecting…", then continues from where it stopped with no prompt on B, and completes. Hashes match. |
| 13 | Pull the USB cable for 10 seconds, then plug it back in. | As in 12. |
| 14 | Lock both screens until the batch ends. | It keeps running, with progress in the notification. |
| 15 | Leave the phones connected and idle for 10 minutes, then add files. | Still connected; the files go straight away. |
| 16 | Force-stop the app on B mid-transfer (`adb shell am force-stop app.nearvolt`). Reopen Receive on B, connect from A again and add the same files. | They resume near where they stopped instead of starting over, and complete with matching hashes. |
| 17 | Force-stop A mid-transfer. Reopen it, connect again and add the same files. | As in 16. |

### Big and many

| # | Steps | Expected |
|---|---|---|
| 18 | Send as much as B's storage allows (aim for 100 GB or more), mixing large videos and thousands of photos. | Every batch ends "Sent and verified". Spot-check hashes. Record the speed below. |
| 19 | Send a folder with nested folders and a few thousand small files. | The same folder structure appears under `Download/Nearvolt`, and the photos show up in the gallery. |

| 19a | Receive a few photos and videos, then open the gallery on B without doing anything else. | They appear. If they don't, a media scan has to be added (not needed so far). |
| 19b | On A, pick a folder with several thousand files. | After a moment a "Reading your files…" notice shows, then the files are listed. |
| 19c | Receive on B while A is on another network: tap Scan code on A, then check B's Wi-Fi Direct group in the log. | A `192.168.49.x` address keeps the group open; any other address closes it. |
| 19d | Check the log after a join on a phone with one Wi-Fi channel. | Two lines, `before joining: router on …` and `after joining: router on …`; note whether the router is lost. |

### Updates

| # | Steps | Expected |
|---|---|---|
| 20 | Install the latest release, open the app and tap Check for updates. | "Nearvolt N is the latest version." Nothing is checked until the button is tapped. |
| 21 | Push a change to `main` and wait for its release. Tap Check for updates. | It downloads version N+1. The first time, Android opens "Install unknown apps"; turn it on and go back, and the update continues by itself. Nearvolt closes for a moment; reopened, it runs N+1. |
| 22 | Repeat 21 on Android 12 or later. | No prompt at all this time. |
| 23 | On B, tap Check for updates and, while it downloads, open Receive and accept a connection from A. | Nothing is installed while connected; the update card says to disconnect first. |
| 23a | Update from the app and wait. | Nearvolt closes for a moment and opens again by itself on the new version. If your phone blocks that, a "Nearvolt updated to N · Tap to open" notification appears and goes away once the app is open. |
| 23b | Scan a code, then look at the recent apps list. | Only one Nearvolt window; no second one for the scanner, before or after scanning. |
| 23c | While connected and idle for 10 minutes, check Settings › Battery for Nearvolt. | Negligible use; the wake lock is held only while data moves (the log's `[speed]` lines). |
| 23d | Close the app on B while connected, and leave it closed for 5 minutes. | A shows "Link dropped · Reconnecting…", then after 5 minutes "Connection failed" with a reason and stops retrying. |
| 24 | On a Samsung phone, turn on Settings › Security and privacy › Auto Blocker, then tap Check for updates. | The card stops spinning and says Auto Blocker stopped the update. |
| 25 | Tap Check for updates again. | No second download: the log says "using the N download from earlier". Turn Auto Blocker off and tap again: it installs. |
| 26 | After an update, open the app once. | The downloaded APK is gone from the app's cache (Settings › Apps › Nearvolt › Storage shows no growth). |

### Speeds

Use the same data for every run: one large video (1 GB or more) as one batch,
then a folder of a few hundred photos as a second batch. Keep the phones about
a metre apart, screens on, both charged above 30%, battery saver off. After each
run, share `nearvolt.log` from both phones.

| # | Link | How to set it up | What the sender's log should say |
|---|---|---|---|
| S1 | Both phones on the same 5 GHz Wi-Fi | Send › Scan code. | `joined the receiver's Wi-Fi Direct group in … s`, `connected … (Wi-Fi Direct)`: the phones go direct even though they share the router |
| S2 | Both on a 2.4 GHz Wi-Fi | As S1. | As S1; the `network:` line shows 2.4 GHz for the router |
| S2a | Shared Wi-Fi only (to compare) | Turn off Wi-Fi Direct on the receiver or use a phone that can't create a group; the log then says `falling back to the shared network`. | `connected … (same Wi-Fi)` |
| S3 | Wi-Fi Direct, no router | Wi-Fi on, but forget or switch off the router on both. Send › Scan code. | `joined the receiver's Wi-Fi Direct group in … s`, `(Wi-Fi Direct)` |
| S4 | Wi-Fi Direct, phones on different networks | Each phone on a different Wi-Fi network. Send › Tap phones. | As S3 |
| S5 | A third phone's hotspot | Both phones join the hotspot. Send › Nearby phones. | `(same Wi-Fi)` |
| S6 | USB cable | Send › USB cable. | `over the USB cable` |

What the log shows for each run:

- `[network]` when connecting and when connected: Wi-Fi generation, band,
  signal (dBm; -30 to -60 is strong, below -75 is weak), the phone's link speed
  to the router, any Wi-Fi Direct group, battery saver and temperature.
- `[speed]` every 5 seconds while files move: progress, the speed over those 5
  seconds, and the same network and phone details at that moment. A dip that
  lines up with a weaker signal points to the radio; one that lines up with
  "hot, may slow down" points to heat; one with no change on either points to
  the storage or the other phone.
- `batch N completed: … average …/s; fastest …/s, slowest …/s`.
- `connection lost: …`, then `connected … again, N s after the link dropped`.
- On the receiver: `kept the group: the sender joined it` or `closed the group:
  the sender came over the router or the cable`, showing which route was used.

| # | Phones (sender → receiver) | Batch | Average MB/s | Fastest / slowest | Notes |
|---|---|---|---|---|---|
| S1 | | video | | | |
| S1 | | photos | | | |
| S2 | | | | | |
| S3 | | | | | |
| S4 | | | | | |
| S5 | | | | | |
| S6 | | | | | |
