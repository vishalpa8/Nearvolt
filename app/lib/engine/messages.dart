import 'package:flutter/services.dart' show PlatformException;

import '../ui/format.dart' show filesLabel;
import 'models.dart' show Batch, BatchState, SetupProblem;
import 'rust/api.dart' show FailureKind;

/// Every failure the app shows is worded here, and only here. The engine reports
/// what happened (a [FailureKind]); this says it in plain words and, where there
/// is one, what to do about it. Technical detail goes to the log, never the screen.

const _generic = 'Something went wrong. Please try again.';

/// A failure of the connection or of a batch.
///
/// [peer] is the other phone's name, if known. [receiving] says whether this phone
/// is the one receiving, because "no space" means a different phone in each case.
String friendlyFailure(FailureKind? kind, {String? peer, bool receiving = false}) {
  final who = (peer == null || peer.trim().isEmpty) ? 'the other phone' : peer.trim();
  final lead = who == 'the other phone' ? 'The other phone' : who;
  return switch (kind) {
    FailureKind.diskFull when receiving =>
      "There isn't enough space on this phone. Free some space, then ask $who to send again.",
    FailureKind.diskFull => "There isn't enough space on $who. Free some space there and send again.",
    FailureKind.cannotSaveFile when receiving =>
      "Nearvolt couldn't save a file on this phone. Check that there's room and that storage access is allowed.",
    FailureKind.cannotSaveFile =>
      "$lead couldn't save a file. Check that their phone has room and that Nearvolt can use its storage.",
    FailureKind.cannotReadFile when receiving => "$lead couldn't read one of the files. Ask them to pick it again.",
    FailureKind.cannotReadFile =>
      "A file couldn't be read. It may have been moved or deleted. Pick it again and retry.",
    FailureKind.fileDamaged =>
      'A file kept arriving damaged. Check the storage on the receiving phone, then send it again.',
    FailureKind.declined => '$lead said no.',
    FailureKind.busy => '$lead is connected to another phone right now. Try again in a moment.',
    FailureKind.otherPhoneGone => "$lead isn't reachable any more. Check that their phone is on and nearby.",
    FailureKind.cannotReach =>
      "Couldn't reach $who. Check that their phone has Receive open, then try Scan code or Tap again.",
    FailureKind.oldVersion => '$lead has an older version of Nearvolt. Update it on both phones.',
    FailureKind.tooBig => "That's too much to send in one go. Try sending fewer files at a time.",
    FailureKind.other || null => _generic,
  };
}

/// An error from the Android side (updates, scanner, permissions). The messages
/// Android code writes for these codes are already in plain words; anything else
/// gets a general sentence rather than an exception's text.
String friendlyPlatform(Object error) {
  const known = {'offline', 'damaged', 'permission', 'busy', 'checksum', 'install', 'scanner', 'no_build'};
  if (error is PlatformException && known.contains(error.code)) {
    final message = error.message;
    if (message != null && message.trim().isNotEmpty) return message;
  }
  return _generic;
}

/// What stops this phone being found while Receive is open, and what still works.
String setupProblemText(SetupProblem problem) => switch (problem) {
  SetupProblem.otherNetworks =>
    "This phone couldn't open its fast Wi-Fi link. Phones on the same Wi-Fi can still connect, but slower.",
};

/// Shown when Receive closed itself after a while with nobody connecting.
const receiveIdleText = "Receive stopped to save battery. Open it again when you're ready.";

/// Shown when picking files finds some that can't be used.
String leftOut(int count) =>
    count == 1 ? "1 file couldn't be read and was left out." : "$count files couldn't be read and were left out.";

/// Shown when the connection ended before files could be queued.
const connectionEnded = 'The connection ended before the files could be sent.';

/// What a connection is called on screen, from the address it really used; null when
/// that isn't known yet.
String? linkName(String? route) {
  if (route == null || route.isEmpty) return null;
  if (route == 'USB cable') return 'USB cable';
  if (route.startsWith('192.168.49.')) return 'Wi-Fi Direct';
  return 'Same Wi-Fi';
}

/// The line under a connection that has ended: what was finished and what was not.
String endedLine({required bool sending, required List<Batch> batches}) {
  final finished = batches.where((b) => b.state == BatchState.done).fold(0, (n, b) => n + b.files);
  final left = batches
      .where((b) => b.state == BatchState.cancelled || b.state == BatchState.failed)
      .fold(0, (n, b) => n + b.files);
  final verb = sending ? 'sent' : 'received';
  if (left == 0) {
    if (finished == 0) return 'Nothing was $verb.';
    return sending ? 'Everything was sent and verified.' : 'Everything was received, verified and saved.';
  }
  final was = left == 1 ? "wasn't" : "weren't";
  if (finished == 0) return 'Cancelled. ${filesLabel(left)} $was $verb.';
  return "${filesLabel(left)} $was $verb. What finished was verified${sending ? '' : ' and saved'}.";
}
