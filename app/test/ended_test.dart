import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';
import 'package:nearvolt/engine/messages.dart';
import 'package:nearvolt/engine/models.dart';
import 'package:nearvolt/pages/connection_page.dart';
import 'package:nearvolt/theme.dart';

import 'fake_engine.dart';

const _done = Batch(id: 0, files: 12, bytes: 48200000, state: BatchState.done);
const _cancelled = Batch(id: 1, files: 4, bytes: 821400000, state: BatchState.cancelled);

void main() {
  group('what an ended connection says', () {
    test('everything finished', () {
      expect(endedLine(sending: true, batches: [_done]), 'Everything was sent and verified.');
      expect(endedLine(sending: false, batches: [_done]), 'Everything was received, verified and saved.');
    });

    test('some finished, some did not', () {
      expect(
        endedLine(sending: true, batches: [_done, _cancelled]),
        "4 files weren't sent. What finished was verified.",
      );
      expect(
        endedLine(sending: false, batches: [_done, _cancelled]),
        "4 files weren't received. What finished was verified and saved.",
      );
    });

    test('one file, and nothing finished', () {
      const one = Batch(id: 0, files: 1, bytes: 10, state: BatchState.cancelled);
      expect(endedLine(sending: true, batches: [one]), "Cancelled. 1 file wasn't sent.");
      expect(endedLine(sending: false, batches: []), 'Nothing was received.');
    });
  });

  test('the connection is named by the route it really took', () {
    expect(linkName('192.168.49.1:4000'), 'Wi-Fi Direct');
    expect(linkName('10.0.0.7'), 'Same Wi-Fi');
    expect(linkName('USB cable'), 'USB cable');
    expect(linkName(''), isNull);
    expect(linkName(null), isNull);
  });

  testWidgets('a file the connection ended on is shown as cancelled, not as still sending', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(),
        home: ConnectionPage(
          engine: FakeEngine(),
          updates: StreamController<Connection>().stream,
          initial: const Connection(
            phase: LinkPhase.ended,
            peer: 'Rahul',
            endedByPeer: true,
            batches: [_done, _cancelled],
          ),
          sending: true,
        ),
      ),
    );
    expect(find.text('Cancelled · not sent'), findsOneWidget);
    expect(find.text('Sending'), findsNothing);
    expect(find.text("4 files weren't sent. What finished was verified."), findsOneWidget);
  });

  testWidgets('the receiving side says not received', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(),
        home: ConnectionPage(
          engine: FakeEngine(),
          updates: StreamController<Connection>().stream,
          initial: const Connection(phase: LinkPhase.ended, peer: 'Rahul', batches: [_cancelled]),
          sending: false,
        ),
      ),
    );
    expect(find.text('Cancelled · not received'), findsOneWidget);
    expect(find.text('Receiving'), findsNothing);
  });
}
