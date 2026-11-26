enum DeviceKind { phone, tablet, laptop }

class NearbyDevice {
  const NearbyDevice({required this.id, required this.name, required this.kind, required this.link});

  final String id;
  final String name;
  final DeviceKind kind;

  /// How the two devices connect, for example "Wi-Fi Direct".
  final String link;
}

class PickedItem {
  const PickedItem({required this.name, required this.bytes, this.files = 1});

  final String name;
  final int bytes;
  final int files;

  bool get isFolder => files > 1;
}

enum TransferPhase { connecting, transferring, reconnecting, completed, failed }

class TransferProgress {
  const TransferProgress({
    required this.phase,
    required this.doneBytes,
    required this.totalBytes,
    required this.doneFiles,
    required this.totalFiles,
    this.bytesPerSecond = 0,
    this.error,
  });

  final TransferPhase phase;
  final int doneBytes;
  final int totalBytes;
  final int doneFiles;
  final int totalFiles;
  final double bytesPerSecond;
  final String? error;

  double get fraction => totalBytes == 0 ? 1 : doneBytes / totalBytes;

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
