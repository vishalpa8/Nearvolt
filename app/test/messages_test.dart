import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter_test/flutter_test.dart';
import 'package:nearvolt/engine/messages.dart';
import 'package:nearvolt/engine/models.dart';
import 'package:nearvolt/engine/rust/api.dart' show FailureKind;

void main() {
  test('every failure kind has a plain sentence with no technical words', () {
    const jargon = ['TLS', 'socket', 'port', 'journal', 'session', 'fingerprint', 'Exception', 'errno', 'close_notify'];
    for (final kind in [...FailureKind.values, null]) {
      for (final receiving in [false, true]) {
        final text = friendlyFailure(kind, peer: "Rahul's Pixel 9", receiving: receiving);
        expect(text, isNotEmpty);
        expect(text, matches(RegExp(r'[.!?]$')), reason: '$kind should read as a sentence');
        for (final word in jargon) {
          expect(text.toLowerCase(), isNot(contains(word.toLowerCase())), reason: '$kind mentions "$word"');
        }
      }
    }
  });

  test('"no space" names the phone that is short of it', () {
    expect(friendlyFailure(FailureKind.diskFull, peer: 'Rahul', receiving: false), contains('on Rahul'));
    expect(friendlyFailure(FailureKind.diskFull, peer: 'Rahul', receiving: true), contains('on this phone'));
  });

  test('without a name the other phone is called that', () {
    expect(friendlyFailure(FailureKind.cannotReach), contains('the other phone'));
    expect(friendlyFailure(FailureKind.otherPhoneGone), startsWith('The other phone'));
    expect(friendlyFailure(FailureKind.declined, peer: '  '), 'The other phone said no.');
  });

  test('platform errors keep their own plain message, and anything else is general', () {
    expect(
      friendlyPlatform(PlatformException(code: 'offline', message: "Couldn't reach GitHub.")),
      "Couldn't reach GitHub.",
    );
    expect(
      friendlyPlatform(PlatformException(code: 'error', message: 'java.lang.NullPointerException at x')),
      isNot(contains('Null')),
    );
    expect(friendlyPlatform(StateError('boom')), 'Something went wrong. Please try again.');
  });

  test('leaving files out is counted in words', () {
    expect(leftOut(1), "1 file couldn't be read and was left out.");
    expect(leftOut(3), "3 files couldn't be read and were left out.");
  });
  group('the transfer notification', () {
    String size(int n) => '$n B';
    TransferNote note(Connection c, {bool sending = true}) => transferNote(c, sending: sending, bytes: size);
    const moving = Batch(id: 0, files: 6, bytes: 600, state: BatchState.moving);

    test('says who it is sending to or receiving from while data moves, the same wording on both phones', () {
      const c = Connection(
        phase: LinkPhase.connected,
        peer: 'Rahul',
        batches: [moving],
        progress: Progress(doneBytes: 300, totalBytes: 600, doneFiles: 2, totalFiles: 6, bytesPerSecond: 50),
      );
      final sender = note(c);
      final receiver = note(c, sending: false);
      expect(sender.title, 'Sending to Rahul');
      expect(receiver.title, 'Receiving from Rahul');
      expect(sender.text, receiver.text);
      expect(sender.text, '2 of 6 files · 300 B of 600 B · 50 B/s');
      expect(sender.percent, 50);
    });

    test('ends as "Sent 6 files" with no bar once the last batch is done', () {
      const c = Connection(
        phase: LinkPhase.connected,
        peer: 'Rahul',
        batches: [Batch(id: 0, files: 6, bytes: 600, state: BatchState.done)],
      );
      expect(note(c).text, 'Sent 6 files');
      expect(note(c).percent, isNull);
      expect(note(c, sending: false).text, 'Received 6 files');
    });

    test('says when the link dropped, and when a batch is waiting', () {
      expect(note(const Connection(phase: LinkPhase.reconnecting, peer: 'Rahul')).title, 'Reconnecting to Rahul');
      const waiting = Connection(
        phase: LinkPhase.connected,
        peer: 'Rahul',
        batches: [Batch(id: 0, files: 1, bytes: 1)],
      );
      expect(note(waiting).text, '1 batch waiting');
    });

    test('only the numbers creeping up share a key, so they can be sent once a second', () {
      const a = Connection(
        phase: LinkPhase.connected,
        peer: 'Rahul',
        batches: [moving],
        progress: Progress(doneBytes: 100, totalBytes: 600, doneFiles: 1, totalFiles: 6),
      );
      final b = a.copyWith(progress: const Progress(doneBytes: 200, totalBytes: 600, doneFiles: 1, totalFiles: 6));
      expect(note(a).key, note(b).key);
      expect(note(a).key, isNot(note(a.copyWith(batches: [moving.copyWith(state: BatchState.done)])).key));
    });
  });
}
