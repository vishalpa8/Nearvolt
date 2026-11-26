import 'models.dart';

/// Everything the UI needs from the transfer engine. The Rust core implements it
/// through flutter_rust_bridge; [DemoEngine] simulates it for UI work.
abstract interface class TransferEngine {
  /// The name nearby devices see.
  String get deviceName;

  /// True when devices and transfers are simulated, so the UI can say so.
  bool get isSimulated;

  Stream<List<NearbyDevice>> nearbyDevices();

  Stream<IncomingOffer> incomingOffers();

  /// Opens the platform's file picker. Returns an empty list if cancelled.
  Future<List<PickedItem>> pickFiles();

  Stream<TransferProgress> send(NearbyDevice to, List<PickedItem> items);

  Stream<TransferProgress> receive(IncomingOffer offer);

  void decline(IncomingOffer offer);
}
