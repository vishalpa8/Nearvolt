enum DeviceKind {
  phone,
  tablet,
  laptop;

  static DeviceKind parse(String? value) => values.firstWhere((k) => k.name == value, orElse: () => phone);
}

/// What a page uses, so it asks Android for that and nothing more.
enum Purpose {
  /// Tapping phones together: NFC only.
  tap,

  /// Scanning the receiver's QR code.
  scan,

  /// Finding phones on the local network and over Bluetooth.
  nearby,

  /// A USB cable.
  usb,

  /// Being found, and receiving.
  receive;

  /// Needs the Nearby devices permission, Bluetooth and, on Android 11 and 12, Location.
  bool get findsPhones => this == nearby || this == receive;
}

/// Something off that a page needs, most important first.
enum Need { permission, permissionInSettings, nfc, wifi, bluetooth, location }

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
  /// demand for devices found over Bluetooth. None for a USB cable.
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

enum BatchState { queued, moving, done, failed }

/// Files added together, sent one batch after another over a connection.
class Batch {
  const Batch({required this.id, required this.files, required this.bytes, this.state = BatchState.queued, this.error});

  /// Counts batches on the connection, from 0.
  final int id;
  final int files;
  final int bytes;
  final BatchState state;
  final String? error;

  Batch copyWith({int? files, int? bytes, BatchState? state, String? error}) => Batch(
    id: id,
    files: files ?? this.files,
    bytes: bytes ?? this.bytes,
    state: state ?? this.state,
    error: error ?? this.error,
  );
}

/// Progress of the batch being sent or received.
class Progress {
  const Progress({
    this.doneBytes = 0,
    this.totalBytes = 0,
    this.doneFiles = 0,
    this.totalFiles = 0,
    this.bytesPerSecond = 0,
  });

  final int doneBytes;
  final int totalBytes;
  final int doneFiles;
  final int totalFiles;
  final double bytesPerSecond;

  double get fraction => totalBytes == 0 ? 0 : (doneBytes / totalBytes).clamp(0, 1);

  Duration? get timeLeft {
    if (bytesPerSecond <= 0) return null;
    return Duration(seconds: ((totalBytes - doneBytes) / bytesPerSecond).ceil());
  }
}

enum LinkPhase {
  connecting,

  /// The receiver is deciding; the sender may show a code to compare.
  waiting,
  connected,
  reconnecting,

  /// Either phone disconnected.
  ended,
  failed,
}

/// A phone asking to connect, on the receiving side.
class Request {
  const Request({required this.from, this.code});

  final String from;

  /// Shown when the phones paired without a QR code or tap; both screens match.
  final String? code;
}

/// One connection, as the UI shows it. Every change arrives as a new snapshot.
class Connection {
  const Connection({
    this.phase = LinkPhase.connecting,
    this.peer = '',
    this.link = '',
    this.code,
    this.request,
    this.ticket,
    this.batches = const [],
    this.progress,
    this.error,
    this.endedByPeer = false,
  });

  final LinkPhase phase;

  /// The other phone's name.
  final String peer;

  /// How the phones are linked, for example "Same Wi-Fi".
  final String link;

  /// Sender: the code to compare while the receiver decides.
  final String? code;

  /// Receiver: a phone asking to connect.
  final Request? request;

  /// Receiver: what the QR code shows while waiting.
  final String? ticket;

  /// Oldest first.
  final List<Batch> batches;

  /// The batch being sent or received.
  final Progress? progress;
  final String? error;
  final bool endedByPeer;

  bool get open => phase != LinkPhase.ended && phase != LinkPhase.failed;

  Batch? get moving => batches.where((b) => b.state == BatchState.moving).firstOrNull;

  Connection copyWith({
    LinkPhase? phase,
    String? peer,
    String? link,
    String? ticket,
    List<Batch>? batches,
    Progress? progress,
    String? error,
    bool? endedByPeer,
    bool keepCode = true,
    String? code,
    bool keepRequest = true,
    Request? request,
    bool keepProgress = true,
  }) => Connection(
    phase: phase ?? this.phase,
    peer: peer ?? this.peer,
    link: link ?? this.link,
    code: code ?? (keepCode ? this.code : null),
    request: request ?? (keepRequest ? this.request : null),
    ticket: ticket ?? this.ticket,
    batches: batches ?? this.batches,
    progress: progress ?? (keepProgress ? this.progress : null),
    error: error ?? this.error,
    endedByPeer: endedByPeer ?? this.endedByPeer,
  );
}
