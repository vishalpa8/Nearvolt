import 'models.dart';

/// Everything the UI needs. [NativeEngine] implements it with the Rust engine and
/// the Android platform layer; tests use a fake.
abstract interface class TransferEngine {
  /// This device. Call once before anything else.
  Future<DeviceProfile> start();

  /// Asks Android for what `purpose` uses, such as the permissions for Wi-Fi Direct,
  /// then returns what's still missing. USB asks for nothing.
  Future<List<Need>> prepare(Purpose purpose);

  /// What `purpose` is missing right now.
  Future<List<Need>> needs(Purpose purpose);

  /// Shows Android's prompt or settings page for `need`; then call [needs].
  Future<void> fix(Need need);

  /// Tickets from phones tapped against this one, while listened to.
  Stream<String> taps();

  /// The phone at the other end of a USB cable, or null, while listened to.
  Stream<Peer?> usbPeer();

  /// The receiver a ticket describes.
  Peer deviceFor(String ticket);

  /// Scans a receiver's QR code. Returns its ticket, or null if cancelled.
  Future<String?> scanCode();

  Future<Selection?> pickFiles();

  Future<Selection?> pickFolder();

  /// Connects to `to` and keeps the connection, reconnecting the same way
  /// whenever the link drops, until [disconnect]. Each event is the whole
  /// connection as it is now.
  Stream<Connection> connect(Peer to);

  /// Cancels a batch, sent or waiting, from this phone. The other phone is told, what
  /// moved of it is thrown away and other batches go on.
  bool cancelBatch(int batch);

  /// Skips one file of a batch (its position in [Batch.rows]) and leaves the rest.
  /// False when the other phone's Nearvolt is too old to do this.
  bool skipFile(int batch, int file);

  /// Adds files to the open connection; they go after anything queued before.
  void queue(Selection what);

  /// Whether what was handed to a connection already. Once it was (sent, cancelled or
  /// failed) its files are released and the pick is used up: it must not be offered again.
  bool taken(Selection what);

  /// Opens Receive until the stream is cancelled: shows a code, then serves one
  /// phone's connection until either side disconnects.
  Stream<Connection> receive();

  /// Answers a phone asking to connect.
  /// Returns false when that question is gone, for example because the phone
  /// asking has already left.
  bool answer(int requestId, {required bool accept});

  /// Ends the open connection; the other phone is told.
  void disconnect();

  /// Opens the system share sheet with 	ext.
  void shareApp(String text);

  /// The version of an available update, if any. Throws when GitHub can't be
  /// reached.
  Future<String?> checkForUpdate();

  /// Downloads and installs the update; reports download progress from 0 to 1.
  Stream<double> installUpdate();
}
