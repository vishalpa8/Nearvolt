import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter_test/flutter_test.dart';
import 'package:nearvolt/engine/messages.dart';
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
}
