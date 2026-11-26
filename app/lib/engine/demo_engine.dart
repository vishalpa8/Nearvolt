import 'dart:async';
import 'dart:math';

import 'engine.dart';
import 'models.dart';

/// Simulated devices and transfers for building the UI before the Rust engine is
/// connected. A transfer plays out in about 20 seconds and includes one dropped
/// link, so the reconnect state is easy to review.
class DemoEngine implements TransferEngine {
  DemoEngine({this.discoveryDelay = const Duration(milliseconds: 300)});

  /// Time between simulated devices appearing.
  final Duration discoveryDelay;

  final _offers = StreamController<IncomingOffer>.broadcast();

  static const _pixel = NearbyDevice(id: 'pixel', name: "Rahul's Pixel 9", kind: DeviceKind.phone, link: 'Wi-Fi Direct');

  static const devices = [
    _pixel,
    NearbyDevice(id: 'ipad', name: "Ananya's iPad", kind: DeviceKind.tablet, link: 'Wi-Fi Aware'),
    NearbyDevice(id: 'studio', name: 'Studio laptop', kind: DeviceKind.laptop, link: 'Same Wi-Fi'),
  ];

  static const sampleItems = [
    PickedItem(name: 'Wedding 2026', bytes: 214800000000, files: 1840),
    PickedItem(name: 'Drone footage', bytes: 96400000000, files: 62),
    PickedItem(name: 'Project archive.zip', bytes: 812000000),
  ];

  static const sampleOffer = IncomingOffer(from: _pixel, files: 248, bytes: 312000000000);

  @override
  String get deviceName => "Satya's laptop";

  @override
  bool get isSimulated => true;

  @override
  Stream<List<NearbyDevice>> nearbyDevices() async* {
    if (discoveryDelay == Duration.zero) {
      yield devices;
      return;
    }
    yield const [];
    for (var i = 1; i <= devices.length; i++) {
      await Future<void>.delayed(discoveryDelay * (i + 1));
      yield devices.sublist(0, i);
    }
  }

  @override
  Stream<IncomingOffer> incomingOffers() => _offers.stream;

  void simulateIncoming() => _offers.add(sampleOffer);

  @override
  Future<List<PickedItem>> pickFiles() async => sampleItems;

  @override
  Stream<TransferProgress> send(NearbyDevice to, List<PickedItem> items) {
    return _simulate(
      items.fold(0, (sum, item) => sum + item.bytes),
      items.fold(0, (sum, item) => sum + item.files),
    );
  }

  @override
  Stream<TransferProgress> receive(IncomingOffer offer) => _simulate(offer.bytes, offer.files);

  @override
  void decline(IncomingOffer offer) {}

  Stream<TransferProgress> _simulate(int totalBytes, int totalFiles) async* {
    TransferProgress at(TransferPhase phase, double fraction, [double speed = 0]) => TransferProgress(
          phase: phase,
          doneBytes: (totalBytes * fraction).round(),
          totalBytes: totalBytes,
          doneFiles: (totalFiles * fraction).floor(),
          totalFiles: totalFiles,
          bytesPerSecond: speed,
        );

    yield at(TransferPhase.connecting, 0);
    await Future<void>.delayed(const Duration(milliseconds: 900));
    final random = Random(7);
    const steps = 200;
    for (var step = 1; step <= steps; step++) {
      if (step == 120) {
        yield at(TransferPhase.reconnecting, (step - 1) / steps);
        await Future<void>.delayed(const Duration(milliseconds: 1800));
      }
      await Future<void>.delayed(const Duration(milliseconds: 100));
      yield at(TransferPhase.transferring, step / steps, 1.05e9 + random.nextDouble() * 1.5e8);
    }
    yield at(TransferPhase.completed, 1);
  }
}
