import 'models.dart';

/// Everything the UI needs. [NativeEngine] implements it with the Rust engine and
/// the Android platform layer; tests use a fake.
abstract interface class TransferEngine {
  /// This device. Call once before anything else.
  Future<DeviceProfile> start();

  /// Asks Android for the permissions Send and Receive use and to turn on
  /// Bluetooth, then returns what's still missing.
  Future<List<Need>> prepare();

  /// What's missing right now.
  Future<List<Need>> needs();

  /// Shows Android's prompt or settings page for `need`, then returns what's
  /// still missing.
  Future<List<Need>> fix(Need need);

  /// Devices with Receive open, while listened to.
  Stream<List<NearbyDevice>> nearby();

  /// Tickets from phones tapped against this one, while the send page listens.
  Stream<String> taps();

  /// The receiver a ticket describes, for showing before sending.
  NearbyDevice deviceFor(String ticket);

  Future<Selection?> pickFiles();

  Future<Selection?> pickFolder();

  /// Scans a receiver's QR code. Returns its ticket, or null if cancelled.
  Future<String?> scanCode();

  /// Sends and keeps going through dropped links until done or stopped.
  Stream<TransferProgress> send(NearbyDevice to, Selection what);

  /// Opens Receive until the stream is cancelled.
  Stream<ReceiveEvent> receive();

  void answer({required bool accept});

  /// Stops the transfer in progress. Received files stay, and it can resume.
  void stopTransfer();

  /// The version of an available update, if any.
  Future<String?> checkForUpdate();

  /// Downloads and installs the update; reports download progress from 0 to 1.
  Stream<double> installUpdate();
}
