@Tags(['golden'])
library;

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

const _connected = Connection(phase: LinkPhase.connected, peer: "Rahul's Pixel 9", link: 'Same Wi-Fi');

const _half = Progress(
  doneBytes: 410700000,
  totalBytes: 821400000,
  doneFiles: 2,
  totalFiles: 4,
  bytesPerSecond: 92000000,
);

const _batches = [
  Batch(id: 0, files: 12, bytes: 48200000, state: BatchState.done),
  Batch(id: 1, files: 4, bytes: 821400000, state: BatchState.moving),
  Batch(id: 2, files: 1, bytes: 2300000),
];

void main() {
  late FakeEngine engine;

  setUp(() => engine = FakeEngine());

  Future<void> phone(WidgetTester tester, Widget app) async {
    tester.view.physicalSize = const Size(780, 1688);
    tester.view.devicePixelRatio = 2;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(app);
    await tester.pump();
  }

  Widget screen(Widget home) => MaterialApp(theme: buildTheme(), debugShowCheckedModeBanner: false, home: home);

  Future<void> shoot(String name) => expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/$name.png'));

  /// Waits a set time rather than settling: searching shows a spinner that never stops.
  Future<void> go(WidgetTester tester, String label) async {
    await tester.tap(find.text(label));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
  }

  testWidgets('home', (tester) async {
    await phone(tester, NearvoltApp(engine: engine, profile: profile));
    await tester.pumpAndSettle();
    await shoot('home');
  });

  testWidgets('update', (tester) async {
    engine.update = '0.1.9';
    await phone(tester, NearvoltApp(engine: engine, profile: profile));
    await tester.tap(find.text('Check for updates'));
    await tester.pump();
    engine.installing.add(0.4);
    await tester.pumpAndSettle();
    await shoot('update');
  });

  testWidgets('send', (tester) async {
    await phone(tester, NearvoltApp(engine: engine, profile: profile));
    await go(tester, 'Send');
    await shoot('send');
  });

  testWidgets('tap', (tester) async {
    await phone(tester, NearvoltApp(engine: engine, profile: profile));
    await go(tester, 'Send');
    await go(tester, 'Tap phones');
    await shoot('tap');
  });

  testWidgets('needs', (tester) async {
    engine.missing = const [Need.permission];
    await phone(tester, NearvoltApp(engine: engine, profile: profile));
    await go(tester, 'Send');
    await go(tester, 'Tap phones');
    await tester.pump(const Duration(milliseconds: 400));
    await shoot('needs');
  });

  Widget connection(Connection c, {bool sending = true}) => screen(
    ConnectionPage(engine: engine, updates: StreamController<Connection>().stream, initial: c, sending: sending),
  );

  for (final (name, c, sending) in [
    (
      'code',
      const Connection(phase: LinkPhase.waiting, peer: "Rahul's Pixel 9", link: 'Same Wi-Fi', code: '4821'),
      true,
    ),
    ('connected', _connected, true),
    ('sending', _connected.copyWith(batches: _batches, progress: _half), true),
    ('reconnecting', _connected.copyWith(phase: LinkPhase.reconnecting, batches: _batches, progress: _half), true),
    ('ended', _connected.copyWith(phase: LinkPhase.ended, endedByPeer: true, batches: _batches.take(1).toList()), true),
    ('received', _connected.copyWith(batches: _batches.take(2).toList(), progress: _half), false),
  ]) {
    testWidgets(name, (tester) async {
      await phone(tester, connection(c, sending: sending));
      await tester.pumpAndSettle();
      await shoot(name);
    });
  }

  testWidgets('disconnect', (tester) async {
    await phone(tester, connection(_connected));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Disconnect'));
    await tester.pumpAndSettle();
    await shoot('disconnect');
  });

  testWidgets('cancelled', (tester) async {
    await phone(
      tester,
      connection(
        const Connection(
          phase: LinkPhase.ended,
          peer: "Rahul's Pixel 9",
          endedByPeer: true,
          batches: [
            Batch(id: 0, files: 12, bytes: 48200000, state: BatchState.done),
            Batch(id: 1, files: 4, bytes: 821400000, state: BatchState.cancelled),
          ],
        ),
      ),
    );
    await tester.pumpAndSettle();
    await shoot('cancelled');
  });

  testWidgets('problems', (tester) async {
    await phone(tester, screen(ReceivePage(engine: engine, profile: profile)));
    engine.receiving!.add(const Connection(ticket: _ticket, problems: [SetupProblem.otherNetworks]));
    await tester.pumpAndSettle();
    await shoot('problems');
  });

  testWidgets('idle', (tester) async {
    await phone(tester, screen(ReceivePage(engine: engine, profile: profile)));
    engine.receiving!.add(const Connection(ticket: _ticket));
    await tester.pumpAndSettle();
    await tester.pump(const Duration(minutes: 11));
    await tester.pumpAndSettle();
    await shoot('idle');
  });

  testWidgets('add', (tester) async {
    await phone(tester, connection(_connected));
    await tester.pumpAndSettle();
    await go(tester, 'Add files');
    await shoot('add');
  });

  testWidgets('receive', (tester) async {
    await phone(tester, screen(ReceivePage(engine: engine, profile: profile)));
    engine.receiving!.add(const Connection(ticket: _ticket));
    await tester.pumpAndSettle();
    await shoot('receive');
  });

  testWidgets('request', (tester) async {
    await phone(tester, screen(ReceivePage(engine: engine, profile: profile)));
    engine.receiving!
      ..add(const Connection(ticket: _ticket))
      ..add(
        const Connection(
          ticket: _ticket,
          phase: LinkPhase.waiting,
          peer: "Rahul's Pixel 9",
          request: Request(id: 1, from: "Rahul's Pixel 9", code: '4821'),
        ),
      );
    await tester.pumpAndSettle();
    await shoot('request');
  });
}
