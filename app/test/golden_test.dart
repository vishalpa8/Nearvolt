@Tags(['golden'])
library;

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';
import 'package:nearvolt/app.dart';
import 'package:nearvolt/engine/models.dart';
import 'package:nearvolt/pages/receive_page.dart';
import 'package:nearvolt/pages/transfer_page.dart';
import 'package:nearvolt/theme.dart';

import 'fake_engine.dart';

const _half = TransferProgress(
  phase: TransferPhase.transferring,
  doneBytes: 410700000,
  totalBytes: 821400000,
  doneFiles: 2,
  totalFiles: 4,
  bytesPerSecond: 92000000,
);

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

  testWidgets('start', (tester) async {
    engine.update = '0.2.0';
    await phone(tester, NearvoltApp(engine: engine, profile: profile));
    engine.devices.add([rahul, tablet]);
    await tester.pumpAndSettle();
    await shoot('start');
  });

  testWidgets('ready', (tester) async {
    await phone(tester, NearvoltApp(engine: engine, profile: profile));
    engine.devices.add([rahul, tablet]);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Choose files'));
    await tester.pumpAndSettle();
    await tester.tap(find.text("Rahul's Pixel 9"));
    await tester.pumpAndSettle();
    await shoot('ready');
  });

  for (final (name, progress) in [
    ('code', const TransferProgress(phase: TransferPhase.waiting, code: '4821')),
    ('sending', _half),
    ('reconnecting', TransferProgress(
      phase: TransferPhase.reconnecting,
      doneBytes: _half.doneBytes,
      totalBytes: _half.totalBytes,
      doneFiles: 2,
      totalFiles: 4,
    )),
    ('done', const TransferProgress(
      phase: TransferPhase.completed,
      doneBytes: 821400000,
      totalBytes: 821400000,
      doneFiles: 4,
      totalFiles: 4,
    )),
  ]) {
    testWidgets(name, (tester) async {
      await phone(
        tester,
        screen(TransferPage(
          progress: Stream.value(progress),
          peerName: rahul.name,
          link: rahul.link,
          incoming: false,
          onStop: () {},
        )),
      );
      await tester.pumpAndSettle();
      await shoot(name);
    });
  }

  testWidgets('receive', (tester) async {
    await phone(tester, screen(ReceivePage(engine: engine, profile: profile)));
    engine.receiving!.add(const ReceiveReady('NV1AEBAGBAFAYDQQCIKBMGA2DQNZXGI3DMOJQGEYTCMBQGAYTANBTGA3TOMBRGU'));
    await tester.pumpAndSettle();
    await shoot('receive');
  });

  testWidgets('offer', (tester) async {
    await phone(tester, screen(ReceivePage(engine: engine, profile: profile)));
    engine.receiving!
      ..add(const ReceiveReady('NV1AEBAGBAFAYDQQCIKBMGA2DQNZXGI3DMOJQGEYTCMBQGAYTANBTGA3TOMBRGU'))
      ..add(const ReceiveOffer(IncomingOffer(from: rahul, files: 4, bytes: 821400000, code: '4821')));
    await tester.pumpAndSettle();
    await shoot('offer');
  });
}
