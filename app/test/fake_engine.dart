import 'dart:async';

import 'package:nearvolt/engine/engine.dart';
import 'package:nearvolt/engine/models.dart';

const profile = DeviceProfile(name: 'Pixel 8', kind: DeviceKind.phone, hasNfc: true, version: '0.1.0');

const rahul = Peer(id: 'a', name: "Rahul's Pixel 9", kind: DeviceKind.phone, link: 'Same Wi-Fi', ticket: 'NV1A');

const album = Selection(
  token: 7,
  files: [
    SelectedFile(path: 'Goa trip/IMG_0001.jpg', size: 4200000),
    SelectedFile(path: 'Goa trip/IMG_0002.jpg', size: 3900000),
    SelectedFile(path: 'Goa trip/clip.mp4', size: 812000000),
    SelectedFile(path: 'Notes.pdf', size: 1300000),
  ],
);

/// A scripted engine: tests push what the platform and Rust would report.
class FakeEngine implements TransferEngine {
  final tapped = StreamController<String>.broadcast();
  final usb = StreamController<Peer?>.broadcast();
  StreamController<Connection>? sending;
  StreamController<Connection>? receiving;
  Peer? connectedTo;
  final queued = <Selection>[];
  bool? answered;
  int? answeredId;
  int disconnects = 0;
  int receiveSessions = 0;
  int scans = 0;
  int updateChecks = 0;
  String? update;
  bool updateFails = false;
  final installing = StreamController<double>();

  /// Which methods asked for setup, in order.
  final prepared = <Purpose>[];

  /// What Android is missing; tests change it to simulate the user fixing things.
  List<Need> missing = const [];
  final fixed = <Need>[];
  bool hangPrepare = false;

  @override
  Future<DeviceProfile> start() async => profile;

  @override
  Future<List<Need>> prepare(Purpose purpose) async {
    prepared.add(purpose);
    // A prompt that never reports back, as when a settings screen is lost.
    if (hangPrepare) return Completer<List<Need>>().future;
    return missing;
  }

  @override
  Future<List<Need>> needs(Purpose purpose) async => missing;

  @override
  Future<void> fix(Need need) async => fixed.add(need);

  @override
  Stream<String> taps() => tapped.stream;

  @override
  Stream<Peer?> usbPeer() => usb.stream;

  @override
  Peer deviceFor(String ticket) => ticket == rahul.ticket
      ? rahul
      : Peer(id: ticket, name: 'Tapped phone', kind: DeviceKind.phone, link: 'NFC', ticket: ticket);

  @override
  Future<Selection?> pickFiles() async => album;

  @override
  Future<Selection?> pickFolder() async => album;

  @override
  Future<String?> scanCode() async {
    scans++;
    return null;
  }

  @override
  Stream<Connection> connect(Peer to) {
    connectedTo = to;
    return (sending = StreamController()).stream;
  }

  final cancelledBatches = <int>[];
  final skippedFiles = <(int, int)>[];
  bool canSkip = true;

  @override
  bool cancelBatch(int batch) {
    cancelledBatches.add(batch);
    return true;
  }

  @override
  bool skipFile(int batch, int file) {
    if (canSkip) skippedFiles.add((batch, file));
    return canSkip;
  }

  final shared = <String>[];

  @override
  void shareApp(String text) => shared.add(text);

  @override
  void queue(Selection what) => queued.add(what);

  @override
  bool taken(Selection what) => queued.contains(what);

  @override
  Stream<Connection> receive() {
    receiveSessions++;
    // Cancelling resolves in the test's zone, like the real engine's async* stream.
    return (receiving = StreamController(onCancel: () async {})).stream;
  }

  @override
  bool answer(int requestId, {required bool accept}) {
    answered = accept;
    answeredId = requestId;
    return true;
  }

  @override
  void disconnect() => disconnects++;

  @override
  Future<String?> checkForUpdate() async {
    updateChecks++;
    if (updateFails) throw StateError('offline');
    return update;
  }

  @override
  Stream<double> installUpdate() => installing.stream;
}
