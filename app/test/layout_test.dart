import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';
import 'package:nearvolt/app.dart';
import 'package:nearvolt/engine/models.dart';
import 'package:nearvolt/pages/connection_page.dart';
import 'package:nearvolt/pages/receive_page.dart';
import 'package:nearvolt/theme.dart';

import 'fake_engine.dart';

const _ticket = 'NV1AEBAGBAFAYDQQCIKBMGA2DQNZXGI3DMOJQGEYTCMBQGAYTANBTGA3TOMBRGU';
const _longName = "Vishal Kumar Prakash Pandey's very long phone name with a number 1234567890";

/// Screens that must fit every kind of phone and every text size: nothing may overflow,
/// be cut off, or become unreachable. A overflow is reported by Flutter as an error,
/// which fails the test.
void main() {
  late FakeEngine engine;

  setUp(() => engine = FakeEngine());

  const sizes = {'small phone': Size(720, 1280), 'landscape': Size(1600, 720), 'tablet': Size(1600, 2560)};
  const scales = [1.0, 1.5, 2.0];

  Widget screen(Widget home) => MaterialApp(theme: buildTheme(), debugShowCheckedModeBanner: false, home: home);

  Future<void> show(WidgetTester tester, Size size, double scale, Widget app) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 2;
    tester.platformDispatcher.textScaleFactorTestValue = scale;
    addTearDown(tester.view.reset);
    addTearDown(tester.platformDispatcher.clearAllTestValues);
    await tester.pumpWidget(app);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
  }

  Future<void> go(WidgetTester tester, String label) async {
    // On a short screen the tile is below the fold: scroll to it first, as a person would.
    final any = find.text(label);
    if (any.evaluate().isEmpty) {
      await tester.scrollUntilVisible(any, 200, scrollable: find.byType(Scrollable).last);
    }
    final target = find.text(label).last;
    await tester.ensureVisible(target);
    await tester.tap(target);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
  }

  final screens = <String, Future<Widget> Function(FakeEngine)>{
    'home': (e) async => NearvoltApp(engine: e, profile: profile),
    'receive with a code': (e) async => screen(ReceivePage(engine: e, profile: profile)),
    'connection, sending': (e) async => screen(
      ConnectionPage(
        engine: e,
        updates: StreamController<Connection>().stream,
        initial: const Connection(
          phase: LinkPhase.connected,
          peer: _longName,
          link: 'Wi-Fi Direct',
          progress: Progress(
            doneBytes: 410700000,
            totalBytes: 821400000,
            doneFiles: 2,
            totalFiles: 4,
            bytesPerSecond: 92000000,
          ),
          batches: [
            Batch(id: 0, files: 12, bytes: 48200000, state: BatchState.done),
            Batch(id: 1, files: 4, bytes: 821400000, state: BatchState.moving),
            Batch(id: 2, files: 1, bytes: 2300000),
          ],
        ),
        sending: true,
      ),
    ),
    'connection, ended with cancelled files': (e) async => screen(
      ConnectionPage(
        engine: e,
        updates: StreamController<Connection>().stream,
        initial: const Connection(
          phase: LinkPhase.ended,
          peer: _longName,
          endedByPeer: true,
          batches: [
            Batch(id: 0, files: 12, bytes: 48200000, state: BatchState.done),
            Batch(id: 1, files: 4, bytes: 821400000, state: BatchState.cancelled),
          ],
        ),
        sending: true,
      ),
    ),
    'connection, failed': (e) async => screen(
      ConnectionPage(
        engine: e,
        updates: StreamController<Connection>().stream,
        initial: const Connection(
          phase: LinkPhase.failed,
          peer: _longName,
          error:
              "Couldn't reach the other phone. Make sure both phones are on the same Wi-Fi, or use Scan code or Tap.",
        ),
        sending: false,
      ),
    ),
  };

  for (final MapEntry(key: where, value: size) in sizes.entries) {
    for (final scale in scales) {
      for (final MapEntry(key: name, value: build) in screens.entries) {
        testWidgets('$name fits a $where at text size $scale', (tester) async {
          await show(tester, size, scale, await build(engine));
          if (name == 'receive with a code') {
            engine.receiving!.add(const Connection(ticket: _ticket));
            await tester.pump();
          }
          expect(tester.takeException(), isNull);
        });
      }

      testWidgets('send and nearby fit a $where at text size $scale', (tester) async {
        await show(tester, size, scale, NearvoltApp(engine: engine, profile: profile));
        await go(tester, 'Send');
        expect(tester.takeException(), isNull, reason: 'the send screen');
        await go(tester, 'Nearby phones');
        engine.devices.add([
          const NearbyDevice(id: 'a', name: _longName, kind: DeviceKind.phone, link: 'Same Wi-Fi', ticket: 'x'),
          const NearbyDevice(id: 'b', name: 'Studio tablet', kind: DeviceKind.tablet, link: 'Bluetooth'),
        ]);
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 400));
        expect(tester.takeException(), isNull, reason: 'the nearby list');
      });
    }
  }
}
