enum DeviceKind {
  phone,
  tablet,
  laptop;

  static DeviceKind parse(String? value) => values.firstWhere((k) => k.name == value, orElse: () => phone);
}

/// Something off that Send and Receive need to reach other phones, most
/// important first.
enum Need { permission, permissionInSettings, wifi, bluetooth, location }

class DeviceProfile {
  const DeviceProfile({required this.name, required this.kind, required this.hasNfc, required this.version});

  final String name;
  final DeviceKind kind;
  final bool hasNfc;

  /// The installed Nearvolt version.
  final String version;
}

/// A device with Receive open.
class NearbyDevice {
  const NearbyDevice({required this.id, required this.name, required this.kind, required this.link, this.ticket});

  final String id;
  final String name;
  final DeviceKind kind;

  /// How the two devices connect, for example "Same Wi-Fi".
  final String link;

  /// Known right away for QR codes, NFC taps and the local network; read on
  /// demand for devices found over Bluetooth.
  final String? ticket;
}

class SelectedFile {
  const SelectedFile({required this.path, required this.size, this.modifiedMs});

  /// Relative, with `/` separators; files in a picked folder start with its name.
  final String path;
  final int size;
  final int? modifiedMs;
}

/// What the user picked. The platform keeps access to the files under `token`.
class Selection {
  const Selection({required this.token, required this.files});

  final int token;
  final List<SelectedFile> files;

  int get bytes => files.fold(0, (sum, f) => sum + f.size);

  /// Files grouped by their top-level folder, for display.
  List<PickedItem> get items {
    final groups = <String, PickedItem>{};
    for (final file in files) {
      final name = file.path.split('/').first;
      final item = groups[name];
      groups[name] = PickedItem(
        name: name,
        bytes: (item?.bytes ?? 0) + file.size,
        files: (item?.files ?? 0) + 1,
        isFolder: file.path.contains('/'),
      );
    }
    return groups.values.toList();
  }
}

class PickedItem {
  const PickedItem({required this.name, required this.bytes, this.files = 1, this.isFolder = false});

  final String name;
  final int bytes;
  final int files;
  final bool isFolder;
}

enum TransferPhase { connecting, waiting, transferring, reconnecting, completed, failed }

class TransferProgress {
  const TransferProgress({
    required this.phase,
    this.doneBytes = 0,
    this.totalBytes = 0,
    this.doneFiles = 0,
    this.totalFiles = 0,
    this.bytesPerSecond = 0,
    this.code,
    this.error,
  });

  final TransferPhase phase;
  final int doneBytes;
  final int totalBytes;
  final int doneFiles;
  final int totalFiles;
  final double bytesPerSecond;

  /// The 4-digit code to compare with the other device, while waiting.
  final String? code;
  final String? error;

  double get fraction => totalBytes == 0 ? (phase == TransferPhase.completed ? 1 : 0) : doneBytes / totalBytes;

  bool get finished => phase == TransferPhase.completed || phase == TransferPhase.failed;

  Duration? get timeLeft {
    if (bytesPerSecond <= 0) return null;
    return Duration(seconds: ((totalBytes - doneBytes) / bytesPerSecond).ceil());
  }
}

class IncomingOffer {
  const IncomingOffer({required this.from, required this.files, required this.bytes, this.code});

  final NearbyDevice from;
  final int files;
  final int bytes;

  /// Shown on both devices when they connected without a QR code or NFC tap.
  final String? code;
}

/// What happens while Receive is open.
sealed class ReceiveEvent {
  const ReceiveEvent();
}

/// Receive is ready; `ticket` is what the QR code shows.
class ReceiveReady extends ReceiveEvent {
  const ReceiveReady(this.ticket);

  final String ticket;
}

class ReceiveOffer extends ReceiveEvent {
  const ReceiveOffer(this.offer);

  final IncomingOffer offer;
}

class ReceiveProgress extends ReceiveEvent {
  const ReceiveProgress(this.progress);

  final TransferProgress progress;
}
