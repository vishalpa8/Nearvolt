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
    this.hasBluetooth,
    this.dataDir,
    this.downloadDir,
    this.journalDir,
    this.versionName,
  );

  String name;

  /// "phone" or "tablet".
  String kind;
  bool hasNfc;
  bool hasBluetooth;
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
  PickResult(this.token, this.files);

  /// Lets the engine open the files later, one at a time.
  int token;
  List<PickedFile> files;
}

class WifiGroup {
  WifiGroup(this.ssid, this.passphrase, this.frequency);

  String ssid;
  String passphrase;
  int frequency;
}

class UpdateInfo {
  UpdateInfo(this.version, this.notes, this.url, this.sha256);

  String version;
  String notes;
  String url;
  String sha256;
}

enum PlatformEventKind {
  /// A device with Receive open was found (`id`, `name`, `deviceKind`, `link`,
  /// and `ticket` when it was found on the local network).
  receiverFound,
  receiverLost,

  /// The phones were tapped together; `ticket` carries the receiver's code.
  ticketTapped,

  /// A USB link to another phone is ready; `fd` is a socket the engine owns.
  usbConnected,
  usbDisconnected,

  /// Update download progress, 0 to 1.
  updateProgress,
}

class PlatformEvent {
  PlatformEvent(this.kind);

  PlatformEventKind kind;
  String? id;
  String? name;
  String? deviceKind;
  String? link;
  String? ticket;
  int? fd;
  double? progress;
}

@HostApi()
abstract class NearvoltPlatform {
  DeviceInfo deviceInfo();

  /// `features`: "nearby" (Wi-Fi Direct, Bluetooth) and "notifications".
  @async
  bool requestPermissions(List<String> features);

  @async
  PickResult? pickFiles();

  @async
  PickResult? pickFolder();

  @async
  String? scanQr();

  /// Opens Receive: advertises the tickets over NFC (private) and the local
  /// network and Bluetooth (public), then creates the Wi-Fi Direct group.
  @async
  WifiGroup? startReceiving(String privateTicket, String publicTicket, int port);

  void updateTickets(String privateTicket, String publicTicket, int port);

  void stopReceiving();

  /// Looks for receivers and listens for NFC taps while the send page is open.
  void startDiscovery();

  void stopDiscovery();

  /// Reads the public ticket of a receiver found over Bluetooth.
  @async
  String readTicket(String receiverId);

  @async
  bool joinWifi(WifiGroup group);

  void leaveWifi();

  void startTransferService(String title);

  void updateTransferService(String text, int percent);

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
