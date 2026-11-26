import 'dart:ui' show Size;

import 'package:flutter_test/flutter_test.dart';
import 'package:nearvolt/app.dart';
import 'package:nearvolt/engine/models.dart';

import 'fake_engine.dart';

void main() {
  late FakeEngine engine;

  setUp(() => engine = FakeEngine());

  Future<void> open(WidgetTester tester) async {
    tester.view.physicalSize = const Size(780, 1688);
    tester.view.devicePixelRatio = 2;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(NearvoltApp(engine: engine, profile: profile));
    await tester.pump();
  }

  testWidgets('sends after the receiver confirms the same code', (tester) async {
    await open(tester);
    expect(find.text('Send to anyone nearby'), findsOneWidget);
    expect(find.text('Looking for devices with Receive open…'), findsOneWidget);

    engine.devices.add([rahul, tablet]);
    await tester.pumpAndSettle();
    expect(find.text('Studio tablet'), findsOneWidget);

    await tester.tap(find.text('Choose files'));
    await tester.pumpAndSettle();
    expect(find.text('Ready to send'), findsOneWidget);
    expect(find.text('4 files'), findsOneWidget);
    expect(find.text('Choose who to send to'), findsOneWidget);

    await tester.tap(find.text("Rahul's Pixel 9"));
    await tester.pumpAndSettle();
    await tester.tap(find.text("Send to Rahul's Pixel 9"));
    await tester.pumpAndSettle();
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.sentTo, rahul);

    engine.sending!.add(const TransferProgress(phase: TransferPhase.waiting, code: '4821'));
    await tester.pumpAndSettle();
    expect(find.text('4 8 2 1'), findsOneWidget);
    expect(find.text("Waiting for Rahul's Pixel 9 to accept"), findsOneWidget);

    engine.sending!.add(const TransferProgress(
      phase: TransferPhase.transferring,
      doneBytes: 410700000,
      totalBytes: 821400000,
      doneFiles: 2,
      totalFiles: 4,
      bytesPerSecond: 92000000,
    ));
    await tester.pumpAndSettle();
    expect(find.text('50%'), findsOneWidget);

    engine.sending!.add(const TransferProgress(
      phase: TransferPhase.completed,
      doneBytes: 821400000,
      totalBytes: 821400000,
      doneFiles: 4,
      totalFiles: 4,
    ));
    await tester.pumpAndSettle();
    expect(find.text('All 4 files verified'), findsOneWidget);

    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    expect(find.text('Send to anyone nearby'), findsOneWidget);
  });

  testWidgets('tapping phones with files chosen starts sending at once', (tester) async {
    await open(tester);
    await tester.tap(find.text('Choose files'));
    await tester.pump(const Duration(milliseconds: 400));

    engine.tapped.add('NV1TAP');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.sentTo?.ticket, 'NV1TAP');
  });

  testWidgets('cancelling a transfer stops the engine', (tester) async {
    await open(tester);
    engine.devices.add([rahul]);
    await tester.pumpAndSettle();
    await tester.tap(find.text("Rahul's Pixel 9"));
    await tester.pumpAndSettle();
    await tester.tap(find.text("Send to Rahul's Pixel 9"));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Stop'));
    await tester.pumpAndSettle();
    expect(engine.stops, 1);
    expect(find.text('Ready to send'), findsOneWidget);
  });

  testWidgets('receives after accepting, then opens Receive again', (tester) async {
    await open(tester);
    await tester.tap(find.text('Receive'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Visible as Pixel 8'), findsOneWidget);
    expect(engine.receiveSessions, 1);

    engine.receiving!.add(const ReceiveReady('NV1ABC'));
    await tester.pumpAndSettle();
    expect(find.bySemanticsLabel('QR code to connect'), findsOneWidget);

    engine.receiving!.add(const ReceiveOffer(IncomingOffer(from: rahul, files: 4, bytes: 821400000, code: '4821')));
    await tester.pumpAndSettle();
    expect(find.text("Rahul's Pixel 9 wants to send you"), findsOneWidget);
    expect(find.text('4 8 2 1'), findsOneWidget);

    await tester.tap(find.text('Accept'));
    await tester.pumpAndSettle();
    expect(engine.answered, isTrue);
    expect(find.text('Receiving from'), findsOneWidget);

    engine.receiving!.add(const ReceiveProgress(TransferProgress(
      phase: TransferPhase.completed,
      doneBytes: 821400000,
      totalBytes: 821400000,
      doneFiles: 4,
      totalFiles: 4,
    )));
    await tester.pumpAndSettle();
    expect(find.text('All 4 files verified'), findsOneWidget);

    await tester.tap(find.text('Done'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Visible as Pixel 8'), findsOneWidget);
    expect(engine.receiveSessions, 2);
  });
}
