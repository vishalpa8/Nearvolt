import 'dart:async';

import 'package:nearvolt/engine/engine.dart';
import 'package:nearvolt/engine/models.dart';

const profile = DeviceProfile(name: 'Pixel 8', kind: DeviceKind.phone, hasNfc: true);

const rahul = NearbyDevice(id: 'a', name: "Rahul's Pixel 9", kind: DeviceKind.phone, link: 'Same Wi-Fi', ticket: 'NV1A');
const tablet = NearbyDevice(id: 'b', name: 'Studio tablet', kind: DeviceKind.tablet, link: 'Bluetooth');

const album = Selection(token: 7, files: [
  SelectedFile(path: 'Goa trip/IMG_0001.jpg', size: 4200000),
  SelectedFile(path: 'Goa trip/IMG_0002.jpg', size: 3900000),
  SelectedFile(path: 'Goa trip/clip.mp4', size: 812000000),
  SelectedFile(path: 'Notes.pdf', size: 1300000),
]);

/// A scripted engine: tests push what the platform and Rust would report.
class FakeEngine implements TransferEngine {
  final devices = StreamController<List<NearbyDevice>>.broadcast();
  final tapped = StreamController<String>.broadcast();
  StreamController<TransferProgress>? sending;
  StreamController<ReceiveEvent>? receiving;
  NearbyDevice? sentTo;
  bool? answered;
  int discoveries = 0;
  int receiveSessions = 0;
  int stops = 0;
  String? update;

  /// What Android is missing; tests change it to simulate the user fixing things.
  List<Need> missing = const [];
  final fixed = <Need>[];

  @override
  Future<DeviceProfile> start() async => profile;

  @override
  Future<List<Need>> prepare() async => missing;

  @override
  Future<List<Need>> needs() async => missing;

  @override
  Future<List<Need>> fix(Need need) async {
    fixed.add(need);
    return missing;
  }

  @override
  Stream<List<NearbyDevice>> nearby() {
    discoveries++;
    return devices.stream;
  }

  @override
  Stream<String> taps() => tapped.stream;

  @override
  NearbyDevice deviceFor(String ticket) =>
      NearbyDevice(id: ticket, name: 'Tapped phone', kind: DeviceKind.phone, link: 'NFC', ticket: ticket);

  @override
  Future<Selection?> pickFiles() async => album;

  @override
  Future<Selection?> pickFolder() async => album;

  @override
  Future<String?> scanCode() async => null;

  @override
  Stream<TransferProgress> send(NearbyDevice to, Selection what) {
    sentTo = to;
    return (sending = StreamController()).stream;
  }

  @override
  Stream<ReceiveEvent> receive() {
    receiveSessions++;
    // Cancelling resolves in the test's zone, like the real engine's async* stream.
    return (receiving = StreamController(onCancel: () async {})).stream;
  }

  @override
  void answer({required bool accept}) => answered = accept;

  @override
  void stopTransfer() => stops++;

  @override
  Future<String?> checkForUpdate() async => update;

  @override
  Stream<double> installUpdate() => const Stream.empty();
}
