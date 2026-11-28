// The one interface between the Flutter app and Android. Regenerate after
// changes with:  dart run pigeon --input pigeons/platform.dart

import 'package:pigeon/pigeon.dart';

@ConfigurePigeon(PigeonOptions(
  dartOut: 'lib/engine/platform.g.dart',
  kotlinOut: 'android/app/src/main/kotlin/app/nearvolt/Platform.g.kt',
  kotlinOptions: KotlinOptions(package: 'app.nearvolt'),
))
class DeviceInfo {
  DeviceInfo(
    this.name,
    this.kind,
    this.hasNfc,
    this.dataDir,
    this.downloadDir,
    this.journalDir,
    this.versionName,
  );

  String name;

  /// "phone" or "tablet".
  String kind;
  bool hasNfc;
  String dataDir;
  String downloadDir;
  String journalDir;
  String versionName;
}

/// One picked file. `path` is relative and includes the picked folder's name.
class PickedFile {
  PickedFile(this.path, this.size, this.modifiedMs);

  String path;
  int size;
  int? modifiedMs;
}

class PickResult {
  PickResult(this.token, this.files, this.skipped);

  /// Lets the engine open the files later, one at a time.
  int token;
  List<PickedFile> files;

  /// Files in the pick that couldn't be read (no size, no access) and were left out.
  int skipped;
}

/// What came of opening Receive: the Wi-Fi Direct group, if one could be made.
class ReceiveSetup {
  ReceiveSetup(this.group);

  WifiGroup? group;
}

class WifiGroup {
  WifiGroup(this.ssid, this.passphrase, this.frequency);

  String ssid;
  String passphrase;
  int frequency;
}

/// What Send and Receive need from Android, as it is right now.
class Readiness {
  Readiness(this.permission, this.permissionInSettings, this.wifi, this.location, this.nfc);

  /// The Nearby devices permissions are granted.
  bool permission;

  /// They were refused for good, so only the app's settings can grant them.
  bool permissionInSettings;

  bool wifi;

  /// Location is on, or not needed (Android 13 and later).
  bool location;

  /// On, or the device has no NFC.
  bool nfc;
}

/// Something Android can be asked to turn on.
enum Requirement {
  /// Nearby devices and notifications, through Android's permission dialogs.
  permissions,

  /// The app's settings page, for permissions refused for good.
  appSettings,
  wifi,
  location,
  nfc,
}

class UpdateInfo {
  UpdateInfo(this.version, this.notes, this.url, this.sha256);

  String version;
  String notes;
  String url;
  String sha256;
}

enum PlatformEventKind {
  /// The phones were tapped together; `ticket` carries the receiver's code.
  ticketTapped,

  /// A USB link to another phone is ready: `fd` is a socket the engine owns,
  /// `host` is true on the phone powering the cable, and `name` is the other
  /// phone's name when the host knows it.
  usbConnected,
  usbDisconnected,

  /// Update download progress, 0 to 1.
  updateProgress,
}

class PlatformEvent {
  PlatformEvent(this.kind);

  PlatformEventKind kind;
  String? name;
  String? ticket;
  int? fd;
  bool? host;
  double? progress;
}

@HostApi()
abstract class NearvoltPlatform {
  DeviceInfo deviceInfo();

  /// Adds a line to the log users can share, Download/Nearvolt/nearvolt.log.
  void log(String tag, String message);

  /// One line on what can slow a transfer down, for the log: the Wi-Fi link
  /// (generation, band, signal, speed), any Wi-Fi Direct group, battery saver
  /// and how hot the phone is.
  @async
  String linkReport();

  Readiness readiness();

  /// Shows Android's prompt or settings page for `requirement` and returns once
  /// the user is back.
  @async
  Readiness fix(Requirement requirement);

  @async
  PickResult? pickFiles();

  @async
  PickResult? pickFolder();

  /// The batch made from this pick is over: forget the files and give back the
  /// access Android granted to them.
  void releaseSelection(int token);

  @async
  String? scanQr();

  /// Opens Receive: serves the tap ticket over NFC, then creates the Wi-Fi Direct group.
  @async
  ReceiveSetup startReceiving(String tapTicket, int port);

  void updateTickets(String tapTicket, int port);

  /// A transfer started: stops serving the tap ticket over NFC and closes the Wi-Fi Direct group unless the sender
  /// joined it. The receiver keeps listening, so the sender can still reconnect.
  ///
  /// `senderAddress` is where the sender connected from, when known: an address of
  /// the group means it came that way.
  void stopAdvertising(String? senderAddress);

  void stopReceiving();

  /// Reads a receiver's ticket when the phones are tapped together.
  void startTapReading();

  void stopTapReading();

  /// Starts noticing USB cables to other phones, reported as `usbConnected` and
  /// `usbDisconnected` events.
  void watchUsb();

  @async
  String? joinWifi(WifiGroup group);

  void leaveWifi();

  /// Opens Android's share sheet with 	ext, so the app can be passed on in any chat.
  void shareApp(String text);

  void startTransferService(String title);

  /// Refreshes the transfer notification. percent is 0 to 100 for a bar, -1 for a bar
  /// that waits, and -2 for no bar at all.
  void updateTransferService(String title, String text, int percent);

  void stopTransferService();

  @async
  UpdateInfo? checkForUpdate();

  @async
  void installUpdate(UpdateInfo update);
}

@FlutterApi()
abstract class NearvoltEvents {
  void onEvent(PlatformEvent event);
}
