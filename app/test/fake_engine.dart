import 'dart:async';

import 'package:flutter/foundation.dart' show ValueNotifier;
import 'package:nearvolt/engine/engine.dart';
import 'package:nearvolt/engine/models.dart';

const profile = DeviceProfile(name: 'Pixel 8', kind: DeviceKind.phone, hasNfc: true, version: '0.1.0');

const rahul = NearbyDevice(
  id: 'a',
  name: "Rahul's Pixel 9",
  kind: DeviceKind.phone,
  link: 'Same Wi-Fi',
  ticket: 'NV1A',
);
const tablet = NearbyDevice(id: 'b', name: 'Studio tablet', kind: DeviceKind.tablet, link: 'Bluetooth');

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
  final devices = StreamController<List<NearbyDevice>>.broadcast();
  final tapped = StreamController<String>.broadcast();
  final usb = StreamController<NearbyDevice?>.broadcast();
  StreamController<Connection>? sending;
  StreamController<Connection>? receiving;
  NearbyDevice? connectedTo;
  final queued = <Selection>[];
  bool? answered;
  int? answeredId;
  int disconnects = 0;
  int discoveries = 0;
  int receiveSessions = 0;
  int scans = 0;
  int updateChecks = 0;
  String? update;
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
  final ValueNotifier<bool> searchFailed = ValueNotifier<bool>(false);

  @override
  Stream<List<NearbyDevice>> nearby() {
    discoveries++;
    return devices.stream;
  }

  @override
  Stream<String> taps() => tapped.stream;

  @override
  Stream<NearbyDevice?> usbPeer() => usb.stream;

  @override
  NearbyDevice deviceFor(String ticket) =>
      NearbyDevice(id: ticket, name: 'Tapped phone', kind: DeviceKind.phone, link: 'NFC', ticket: ticket);

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
  Stream<Connection> connect(NearbyDevice to) {
    connectedTo = to;
    return (sending = StreamController()).stream;
  }

  @override
  void queue(Selection what) => queued.add(what);

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
    return update;
  }

  @override
  Stream<double> installUpdate() => installing.stream;
}
