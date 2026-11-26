import 'dart:ui' show Size;

import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart' show LinearProgressIndicator;
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

  Future<void> openSend(WidgetTester tester) async {
    await open(tester);
    await tester.tap(find.text('Send'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
  }

  Future<void> chooseFiles(WidgetTester tester) async {
    await tester.tap(find.text('Choose files'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    await tester.tap(find.text('Files'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
  }

  testWidgets('a failed start explains itself instead of a blank screen', (tester) async {
    await tester.pumpWidget(const StartupFailure(reason: 'the engine library is missing'));
    expect(find.text("Nearvolt couldn't start"), findsOneWidget);
    expect(find.text('the engine library is missing'), findsOneWidget);
  });

  testWidgets('nothing looks for phones until Send or Receive is chosen', (tester) async {
    await open(tester);
    expect(find.text('Move files to anyone nearby'), findsOneWidget);
    expect(engine.discoveries, 0);
    expect(engine.receiveSessions, 0);

    await tester.tap(find.text('Send'));
    await tester.pump();
    expect(engine.discoveries, 1);
  });

  testWidgets('checks for updates only when asked', (tester) async {
    await open(tester);
    expect(engine.updateChecks, 0);

    await tester.tap(find.text('Check for updates'));
    await tester.pump();
    await tester.pump();
    expect(engine.updateChecks, 1);
    expect(find.text('Nearvolt 0.1.0 is the latest version.'), findsOneWidget);

    await tester.tap(find.byTooltip('Close'));
    await tester.pump();
    expect(find.text('Nearvolt 0.1.0 is the latest version.'), findsNothing);
  });

  testWidgets('a new version downloads and installs without another tap', (tester) async {
    engine.update = '0.1.9';
    await open(tester);
    await tester.tap(find.text('Check for updates'));
    await tester.pump();
    await tester.pump();
    expect(find.text('Downloading version 0.1.9…'), findsOneWidget);

    engine.installing.add(0.5);
    await tester.pump();
    expect(tester.widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator)).value, 0.5);

    await engine.installing.close();
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('Installing version 0.1.9'), findsOneWidget);
  });

  testWidgets('sends after the receiver confirms the same code', (tester) async {
    await openSend(tester);
    expect(find.text('Looking for devices with Receive open…'), findsOneWidget);

    engine.devices.add([rahul, tablet]);
    await tester.pumpAndSettle();
    expect(find.text('Studio tablet'), findsOneWidget);

    await chooseFiles(tester);
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
    expect(find.text('Nearby'), findsOneWidget);
  });

  testWidgets('tapping phones with files chosen starts sending at once', (tester) async {
    await openSend(tester);
    await chooseFiles(tester);

    engine.tapped.add('NV1TAP');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.sentTo?.ticket, 'NV1TAP');
  });

  testWidgets('cancelling a transfer stops the engine', (tester) async {
    await openSend(tester);
    engine.devices.add([rahul]);
    await tester.pumpAndSettle();
    await tester.tap(find.text("Rahul's Pixel 9"));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    await tester.tap(find.text('Files'));
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

  testWidgets('a missing permission shows a notice, and fixing it restarts the search', (tester) async {
    engine.missing = const [Need.permission, Need.bluetooth];
    await openSend(tester);
    expect(find.text('Allow Nearby devices so Nearvolt can find phones and be found.'), findsOneWidget);
    expect(engine.discoveries, 1);

    engine.missing = const [];
    await tester.tap(find.text('Allow'));
    await tester.pump();
    expect(engine.fixed, [Need.permission]);
    expect(find.text('Allow'), findsNothing);
    expect(engine.discoveries, 2);
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
