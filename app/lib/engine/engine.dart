import 'package:flutter/foundation.dart' show ValueListenable;

import 'models.dart';

/// Everything the UI needs. [NativeEngine] implements it with the Rust engine and
/// the Android platform layer; tests use a fake.
abstract interface class TransferEngine {
  /// This device. Call once before anything else.
  Future<DeviceProfile> start();

  /// Asks Android for what `purpose` uses, such as permissions and Bluetooth, then
  /// returns what's still missing. Tapping, scanning and USB ask for nothing.
  Future<List<Need>> prepare(Purpose purpose);

  /// What `purpose` is missing right now.
  Future<List<Need>> needs(Purpose purpose);

  /// Shows Android's prompt or settings page for `need`; then call [needs].
  Future<void> fix(Need need);

  /// Devices with Receive open on the local network or over Bluetooth, while listened to.
  Stream<List<NearbyDevice>> nearby();

  /// True when searching the local network couldn't start, even after retrying;
  /// the Nearby page then says so and points to the other ways to connect.
  ValueListenable<bool> get searchFailed;

  /// Tickets from phones tapped against this one, while listened to.
  Stream<String> taps();

  /// The phone at the other end of a USB cable, or null, while listened to.
  Stream<NearbyDevice?> usbPeer();

  /// The receiver a ticket describes.
  NearbyDevice deviceFor(String ticket);

  /// Scans a receiver's QR code. Returns its ticket, or null if cancelled.
  Future<String?> scanCode();

  Future<Selection?> pickFiles();

  Future<Selection?> pickFolder();

  /// Connects to `to` and keeps the connection, reconnecting the same way
  /// whenever the link drops, until [disconnect]. Each event is the whole
  /// connection as it is now.
  Stream<Connection> connect(NearbyDevice to);

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

  /// The version of an available update, if any. Throws when GitHub can't be
  /// reached.
  Future<String?> checkForUpdate();

  /// Downloads and installs the update; reports download progress from 0 to 1.
  Stream<double> installUpdate();
}
