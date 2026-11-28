import 'dart:ui' show AppLifecycleState, Size;

import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart'
    show CircularProgressIndicator, LinearProgressIndicator, NavigatorState, Navigator, TextButton;
import 'package:nearvolt/app.dart';
import 'package:nearvolt/debug.dart';
import 'package:nearvolt/engine/messages.dart';
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

  Future<void> go(WidgetTester tester, String label) async {
    await tester.tap(find.text(label).last);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
  }

  Future<void> openMethod(WidgetTester tester, String method) async {
    await open(tester);
    await go(tester, 'Send');
    await go(tester, method);
  }

  const rahulName = "Rahul's Pixel 9";
  const connected = Connection(phase: LinkPhase.connected, peer: rahulName, link: 'Same Wi-Fi');

  testWidgets('a failed start explains itself instead of a blank screen', (tester) async {
    await tester.pumpWidget(const StartupFailure(reason: 'the engine library is missing'));
    expect(find.text("Nearvolt couldn't start"), findsOneWidget);
    expect(find.text('the engine library is missing'), findsOneWidget);
  });

  testWidgets('nothing starts until a way to connect is chosen', (tester) async {
    await open(tester);
    expect(find.text('Move files to anyone nearby'), findsOneWidget);

    await go(tester, 'Send');
    expect(find.text('How do you want to connect?'), findsOneWidget);
    for (final method in ['Tap phones', 'Scan code', 'USB cable']) {
      expect(find.text(method), findsOneWidget);
    }
    expect(engine.prepared, isEmpty);
    expect(engine.receiveSessions, 0);
    expect(engine.tapped.hasListener, isFalse);

    await go(tester, 'Tap phones');
    expect(engine.prepared, [Purpose.tap]);
    expect(engine.tapped.hasListener, isTrue);
  });

  testWidgets('Share opens the share sheet with where to get Nearvolt', (tester) async {
    await open(tester);
    await tester.tap(find.byTooltip('Share Nearvolt'));
    await tester.pump();
    expect(engine.shared, [shareText]);
    expect(shareText, contains('https://github.com/vishalpa8/Nearvolt/releases/latest'));
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

  testWidgets('the "latest version" notice goes away by itself after a few seconds', (tester) async {
    await open(tester);
    await tester.tap(find.text('Check for updates'));
    await tester.pump();
    await tester.pump();
    expect(find.text('Nearvolt 0.1.0 is the latest version.'), findsOneWidget);

    await tester.pump(const Duration(seconds: 3));
    expect(find.text('Nearvolt 0.1.0 is the latest version.'), findsOneWidget, reason: 'long enough to read');
    await tester.pump(const Duration(seconds: 2));
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Nearvolt 0.1.0 is the latest version.'), findsNothing);
  });

  testWidgets('a failed update stays until the user closes it', (tester) async {
    engine.update = '0.1.9';
    await open(tester);
    await tester.tap(find.text('Check for updates'));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.text('Update'));
    await tester.pump();
    engine.installing.addError(PlatformException(code: 'install', message: 'Android refused the update.'));
    await tester.pump();
    await tester.pump();
    await tester.pump(const Duration(seconds: 30));
    expect(find.text('Android refused the update.'), findsOneWidget);
  });

  group('the background update check', () {
    testWidgets('looks a few seconds after opening, and says nothing when up to date', (tester) async {
      await open(tester);
      expect(engine.updateChecks, 0);
      await tester.pump(const Duration(seconds: 7));
      expect(engine.updateChecks, 1);
      expect(find.textContaining('is available'), findsNothing);
      expect(find.text('Nearvolt 0.1.0 is the latest version.'), findsNothing, reason: 'a quiet check shows nothing');
    });

    testWidgets('a new version is offered, and nothing downloads until the user says Update', (tester) async {
      engine.update = '0.1.9';
      await open(tester);
      await tester.pump(const Duration(seconds: 7));
      await tester.pump();
      expect(find.text('Nearvolt 0.1.9 is available. Download and install it now?'), findsOneWidget);
      expect(find.text('Nearvolt 0.1.9 is available. Download and install it now?'), findsOneWidget);
      expect(find.text('Downloading version 0.1.9…'), findsNothing);

      await tester.tap(find.text('Update'));
      await tester.pump();
      await tester.pump();
      expect(find.text('Nearvolt 0.1.9 is available. Download and install it now?'), findsNothing);
      expect(find.text('Downloading version 0.1.9…'), findsOneWidget);
    });

    testWidgets('Later hides the offer, and the same version is not offered again', (tester) async {
      engine.update = '0.1.9';
      await open(tester);
      await tester.pump(const Duration(seconds: 7));
      await tester.pump();
      await tester.tap(find.text('Later'));
      await tester.pump();
      expect(find.text('Nearvolt 0.1.9 is available. Download and install it now?'), findsNothing);

      await tester.pump(const Duration(hours: 7));
      await tester.pump();
      expect(engine.updateChecks, 2, reason: 'it keeps looking');
      expect(
        find.text('Nearvolt 0.1.9 is available. Download and install it now?'),
        findsNothing,
        reason: 'but does not nag about the same one',
      );

      engine.update = '0.2.0';
      await tester.pump(const Duration(hours: 7));
      await tester.pump();
      expect(
        find.text('Nearvolt 0.2.0 is available. Download and install it now?'),
        findsOneWidget,
        reason: 'a newer one is new news',
      );
    });

    testWidgets('no internet is not worth an error: nothing shows, and it tries again later', (tester) async {
      engine.updateFails = true;
      await open(tester);
      await tester.pump(const Duration(seconds: 7));
      await tester.pump();
      expect(find.textContaining('available'), findsNothing);
      expect(find.byTooltip('Close'), findsNothing);

      engine.updateFails = false;
      engine.update = '0.1.9';
      await tester.pump(const Duration(hours: 7));
      await tester.pump();
      expect(find.text('Nearvolt 0.1.9 is available. Download and install it now?'), findsOneWidget);
    });

    testWidgets('checking by hand asks too, and Later leaves everything as it was', (tester) async {
      engine.update = '0.1.9';
      await open(tester);
      await tester.tap(find.text('Check for updates'));
      await tester.pump();
      await tester.pump();
      expect(find.text('Nearvolt 0.1.9 is available. Download and install it now?'), findsOneWidget);
      expect(tester.takeException(), isNull);

      await tester.tap(find.text('Later'));
      await tester.pump();
      expect(find.textContaining('is available'), findsNothing);
      expect(find.text('Downloading version 0.1.9…'), findsNothing);
      expect(tester.widget<TextButton>(find.widgetWithText(TextButton, 'Check for updates')).onPressed, isNotNull);
    });
  });

  testWidgets('a new version is offered first, and only Update downloads and installs it', (tester) async {
    engine.update = '0.1.9';
    await open(tester);
    await tester.tap(find.text('Check for updates'));
    await tester.pump();
    await tester.pump();
    expect(find.text('Nearvolt 0.1.9 is available. Download and install it now?'), findsOneWidget);
    expect(find.text('Downloading version 0.1.9…'), findsNothing, reason: 'nothing starts by itself');

    await tester.tap(find.text('Update'));
    await tester.pump();
    expect(find.text('Downloading version 0.1.9…'), findsOneWidget);

    engine.installing.add(0.5);
    await tester.pump();
    expect(tester.widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator)).value, 0.5);

    engine.installing.add(1);
    await tester.pump();
    expect(find.textContaining('Installing version 0.1.9'), findsOneWidget);
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
  });

  testWidgets('an install Android refuses stops the spinner and says why', (tester) async {
    engine.update = '0.1.9';
    await open(tester);
    await tester.tap(find.text('Check for updates'));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.text('Update'));
    await tester.pump();
    engine.installing.add(1);
    await tester.pump();

    engine.installing.addError(
      PlatformException(code: 'install', message: "Samsung's Auto Blocker stopped the update."),
    );
    await tester.pump();
    await tester.pump();
    expect(find.text("Samsung's Auto Blocker stopped the update."), findsOneWidget);
    expect(find.byType(CircularProgressIndicator), findsNothing);
    expect(find.byTooltip('Close'), findsOneWidget);
  });

  testWidgets('stays connected and sends each batch added, until Disconnect', (tester) async {
    await openMethod(tester, 'Tap phones');
    engine.tapped.add(rahul.ticket!);
    await tester.pumpAndSettle();
    expect(engine.connectedTo, rahul);
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.tapped.hasListener, isFalse, reason: 'no listening for taps while connected');

    engine.sending!.add(const Connection(phase: LinkPhase.waiting, peer: rahulName, code: '4821'));
    await tester.pumpAndSettle();
    expect(find.text('4 8 2 1'), findsOneWidget);
    expect(find.text("Waiting for $rahulName to accept"), findsOneWidget);

    engine.sending!.add(connected);
    await tester.pumpAndSettle();
    expect(find.text('Add files to send them to $rahulName.'), findsOneWidget);

    await go(tester, 'Add files');
    await tester.tap(find.text('Files').last);
    await tester.pumpAndSettle();
    expect(engine.queued, [album]);

    engine.sending!.add(
      connected.copyWith(
        batches: const [Batch(id: 0, files: 4, bytes: 821400000, state: BatchState.moving)],
        progress: const Progress(
          doneBytes: 410700000,
          totalBytes: 821400000,
          doneFiles: 2,
          totalFiles: 4,
          bytesPerSecond: 92000000,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('50%'), findsOneWidget);

    engine.sending!.add(
      connected.copyWith(
        batches: const [
          Batch(id: 0, files: 4, bytes: 821400000, state: BatchState.done),
          Batch(id: 1, files: 1, bytes: 2000000),
        ],
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Sent and verified'), findsOneWidget);
    expect(find.text('Waiting its turn'), findsOneWidget);

    // Back asks first, and staying keeps the connection.
    tester.state<NavigatorState>(find.byType(Navigator)).maybePop();
    await tester.pumpAndSettle();
    expect(find.text('Disconnect from $rahulName?'), findsOneWidget);
    await tester.tap(find.text('Stay connected'));
    await tester.pumpAndSettle();
    expect(engine.disconnects, 0);

    await tester.tap(find.text('Disconnect'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Disconnect').last);
    await tester.pumpAndSettle();
    expect(engine.disconnects, 1);

    engine.sending!.add(connected.copyWith(phase: LinkPhase.ended));
    await tester.pumpAndSettle();
    expect(find.text('Disconnected'), findsOneWidget);
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    expect(find.text('Hold the backs of the phones together'), findsOneWidget);
    expect(engine.tapped.hasListener, isTrue, reason: 'listening for taps again once disconnected');
  });

  Future<void> connectByCode(WidgetTester tester) async {
    await open(tester);
    await go(tester, 'Send');
    await go(tester, 'Choose files');
    await go(tester, 'Files');
    await go(tester, 'Tap phones');
    engine.tapped.add(rahul.ticket!);
    await tester.pumpAndSettle();
    engine.sending!.add(connected.copyWith(code: '4821'));
    await tester.pumpAndSettle();
  }

  testWidgets('a code is shown while the receiver decides, then files go with no second check', (tester) async {
    await connectByCode(tester);
    // The receiver accepted: the sender goes straight on, with nothing more to tap.
    expect(find.text('It matches'), findsNothing);
    expect(find.text("It doesn't match"), findsNothing);
    expect(engine.queued, [album]);
    expect(find.text('Add files'), findsOneWidget);
  });

  const goaRows = [
    BatchFile(name: 'Goa trip/IMG_0001.jpg', size: 4200000, state: FileState.done, bytes: 4200000),
    BatchFile(name: 'Goa trip/clip.mp4', size: 812000000, state: FileState.moving, bytes: 203000000),
    BatchFile(name: 'Notes.pdf', size: 1300000),
  ];
  final goaBatch = Batch(id: 0, files: 3, bytes: 817500000, state: BatchState.moving, rows: goaRows);

  testWidgets('every file of a batch is listed by name, the one moving with a bar and how far it is', (tester) async {
    await connectByCode(tester);
    engine.sending!.add(connected.copyWith(batches: [goaBatch]));
    await tester.pumpAndSettle();

    expect(find.text('IMG_0001.jpg'), findsOneWidget);
    expect(find.text('clip.mp4'), findsOneWidget);
    expect(find.text('Notes.pdf'), findsOneWidget);
    expect(find.text('203 MB of 812 MB'), findsOneWidget);
    expect(find.textContaining('· waiting'), findsOneWidget);
    expect(
      tester.widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator).last).value,
      closeTo(0.25, 0.01),
    );
    // A finished file has nothing to skip.
    expect(find.byTooltip('Skip IMG_0001.jpg'), findsNothing);
    expect(find.byTooltip('Skip clip.mp4'), findsOneWidget);
  });

  testWidgets('skipping one file asks first, then tells only that file to stop', (tester) async {
    await connectByCode(tester);
    engine.sending!.add(connected.copyWith(batches: [goaBatch]));
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Skip clip.mp4'));
    await tester.pumpAndSettle();
    expect(find.text('Skip clip.mp4?'), findsOneWidget);
    expect(find.text('Only this file is left out. The other files carry on.'), findsOneWidget);

    await tester.tap(find.text('Keep going'));
    await tester.pumpAndSettle();
    expect(engine.skippedFiles, isEmpty, reason: 'backing out changes nothing');

    await tester.tap(find.byTooltip('Skip clip.mp4'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Skip file'));
    await tester.pumpAndSettle();
    expect(engine.skippedFiles, [(0, 1)]);
    expect(engine.cancelledBatches, isEmpty);

    engine.sending!.add(
      connected.copyWith(
        batches: [
          goaBatch.copyWith(
            rows: [
              goaRows[0],
              goaRows[1].copyWith(state: FileState.skipped),
              goaRows[2],
            ],
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Skipped · not sent'), findsOneWidget);
    expect(find.byTooltip('Skip clip.mp4'), findsNothing);
  });

  testWidgets('a phone that cannot skip a single file says so, and the batch can still be cancelled', (tester) async {
    engine.canSkip = false;
    await connectByCode(tester);
    engine.sending!.add(connected.copyWith(batches: [goaBatch]));
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Skip Notes.pdf'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Skip file'));
    await tester.pumpAndSettle();
    expect(find.textContaining('needs the latest Nearvolt'), findsOneWidget);
  });

  testWidgets('cancelling a batch asks first and leaves the other batches alone', (tester) async {
    await connectByCode(tester);
    engine.sending!.add(
      connected.copyWith(
        batches: [
          goaBatch,
          const Batch(id: 1, files: 1, bytes: 2000000, rows: [BatchFile(name: 'Later.txt', size: 2000000)]),
        ],
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Cancel batch'), findsNWidgets(2));

    await tester.tap(find.text('Cancel batch').last);
    await tester.pumpAndSettle();
    expect(find.text('Cancel this batch?'), findsOneWidget);
    await tester.tap(find.text('Keep going'));
    await tester.pumpAndSettle();
    expect(engine.cancelledBatches, isEmpty);

    await tester.tap(find.text('Cancel batch').first);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(TextButton, 'Cancel batch').last);
    await tester.pumpAndSettle();
    expect(engine.cancelledBatches.length, 1);
  });

  testWidgets('a connection that has ended offers nothing to cancel or skip', (tester) async {
    await connectByCode(tester);
    engine.sending!.add(
      connected.copyWith(
        phase: LinkPhase.ended,
        batches: [goaBatch.copyWith(state: BatchState.cancelled)],
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Cancel batch'), findsNothing);
    expect(find.byTooltip('Skip clip.mp4'), findsNothing);
  });

  testWidgets('a connection screen whose events stop does not keep saying Connected', (tester) async {
    await connectByCode(tester);
    expect(find.text('Disconnect'), findsWidgets);

    // The engine behind the connection is gone: no more events, and none said it ended.
    await engine.sending!.close();
    await tester.pumpAndSettle();
    expect(find.text('Disconnected'), findsOneWidget);
    expect(find.text('Done'), findsOneWidget);
  });

  testWidgets('a phone connecting while Receive slides away does not open a connection screen', (tester) async {
    await open(tester);
    await go(tester, 'Receive');
    await tester.pump();

    tester.state<NavigatorState>(find.byType(Navigator).first).pop();
    engine.receiving!.add(connected);
    await tester.pump();
    await tester.pumpAndSettle();
    expect(find.text('Receiving from'), findsNothing);
  });
  testWidgets('files that were sent are no longer selected, and are not offered again', (tester) async {
    await connectByCode(tester);
    expect(engine.queued, [album]);

    // The connection ends, and the person connects again from the same list.
    engine.sending!.add(connected.copyWith(phase: LinkPhase.ended));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    engine.tapped.add(rahul.ticket!);
    await tester.pumpAndSettle();
    engine.sending!.add(connected);
    await tester.pumpAndSettle();
    expect(engine.queued, [album], reason: 'the used pick is not sent a second time');

    // Back on the Send screen nothing is selected any more.
    engine.sending!.add(connected.copyWith(phase: LinkPhase.ended));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    tester.state<NavigatorState>(find.byType(Navigator)).pop();
    await tester.pumpAndSettle();
    expect(find.text('Choose files'), findsOneWidget);
    expect(find.text('Change'), findsNothing);
  });
  testWidgets('Cancel before the other phone accepts leaves the screen at once', (tester) async {
    await openMethod(tester, 'Tap phones');
    engine.tapped.add(rahul.ticket!);
    await tester.pumpAndSettle();
    expect(find.text('Connecting…'), findsOneWidget);

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(engine.disconnects, 1);
    expect(find.text('Connecting…'), findsNothing);
    expect(
      find.text('Hold the backs of the phones together'),
      findsOneWidget,
      reason: 'back on the Tap page, not stuck on "Cancelling…"',
    );
  });

  testWidgets('a second tap while a way to connect is opening does nothing', (tester) async {
    await open(tester);
    await go(tester, 'Send');
    await tester.tap(find.text('Tap phones'));
    await tester.tap(find.text('Tap phones'), warnIfMissed: false);
    await tester.pumpAndSettle();
    expect(find.text('Hold the backs of the phones together'), findsOneWidget);
    await tester.tap(find.byTooltip('Back'));
    await tester.pumpAndSettle();
    expect(
      find.text('How do you want to connect?'),
      findsOneWidget,
      reason: 'one Back leaves it: only one page was opened',
    );
  });

  testWidgets('tapping Check for updates again while one is running does nothing', (tester) async {
    engine.update = '0.1.9';
    await open(tester);
    await tester.tap(find.text('Check for updates'));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.text('Check for updates'), warnIfMissed: false);
    await tester.pump();
    expect(engine.updateChecks, 1);
  });

  testWidgets('Receive says what stops other phones finding it, and what still works', (tester) async {
    await open(tester);
    await go(tester, 'Receive');
    engine.receiving!.add(const Connection(ticket: 'NV1ABC', problems: [SetupProblem.otherNetworks]));
    await tester.pumpAndSettle();
    expect(find.text(setupProblemText(SetupProblem.otherNetworks)), findsOneWidget);
  });

  testWidgets('Receive closes itself after ten quiet minutes and opens again on request', (tester) async {
    await open(tester);
    await go(tester, 'Receive');
    engine.receiving!.add(const Connection(ticket: 'NV1ABC'));
    await tester.pumpAndSettle();
    expect(engine.receiveSessions, 1);

    await tester.pump(const Duration(minutes: 9));
    expect(find.text(receiveIdleText), findsNothing);
    await tester.pump(const Duration(minutes: 2));
    await tester.pump();
    expect(find.text(receiveIdleText), findsOneWidget);
    expect(find.bySemanticsLabel('QR code to connect'), findsNothing);

    await tester.tap(find.text('Open Receive again'));
    await tester.pump();
    expect(engine.receiveSessions, 2);
    expect(find.text(receiveIdleText), findsNothing);
  });

  testWidgets('tapping phones connects at once', (tester) async {
    await openMethod(tester, 'Tap phones');
    expect(find.text('Hold the backs of the phones together'), findsOneWidget);

    engine.tapped.add('NV1TAP');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.connectedTo?.ticket, 'NV1TAP');
    expect(engine.tapped.hasListener, isFalse);
  });

  testWidgets('a USB cable connects as soon as the other phone answers', (tester) async {
    await openMethod(tester, 'USB cable');
    expect(engine.prepared, [Purpose.usb]);

    engine.usb.add(const Peer(id: 'usb', name: 'USB cable', kind: DeviceKind.phone, link: 'USB cable'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.connectedTo?.id, 'usb');
  });

  testWidgets('a missing permission closes the page: nothing starts until it is allowed', (tester) async {
    engine.missing = const [Need.permission, Need.location];
    await openMethod(tester, 'Tap phones');
    expect(find.text('Turn these on to continue'), findsOneWidget);
    expect(find.text('Allow Nearby devices so Nearvolt can open its fast Wi-Fi link between phones.'), findsOneWidget);
    expect(find.text('This Android version needs Location on to connect phones.'), findsOneWidget);
    expect(engine.tapped.hasListener, isFalse);

    engine.missing = const [];
    await tester.tap(find.text('Allow'));
    await tester.pump();
    expect(engine.fixed, [Need.permission]);
    expect(find.text('Turn these on to continue'), findsNothing);
    expect(engine.tapped.hasListener, isTrue);
  });

  testWidgets('a prompt that never answers does not leave the page blank for ever', (tester) async {
    engine.hangPrepare = true;
    await openMethod(tester, 'Tap phones');
    expect(find.text('Turn these on to continue'), findsNothing);

    // Coming back to the app checks again, even though the first check never ended.
    engine.missing = const [Need.location];
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Turn these on to continue'), findsOneWidget);

    // The abandoned first check gives up on its own later, and changes nothing.
    await tester.pump(const Duration(seconds: 91));
    expect(find.text('Turn these on to continue'), findsOneWidget);
  });

  testWidgets('every tap, screen and message is written to the log', (tester) async {
    final lines = <String>[];
    DebugLog.sink = (tag, message) => lines.add('[$tag] $message');
    addTearDown(() => DebugLog.sink = null);

    await open(tester);
    await go(tester, 'Send');
    await go(tester, 'Tap phones');

    expect(lines.where((l) => l.startsWith('[tap]')), isNotEmpty, reason: lines.join('\n'));
    expect(lines.any((l) => l.startsWith('[tap] "Send')), isTrue, reason: lines.join('\n'));
    expect(lines.any((l) => l.startsWith('[tap] "Tap phones')), isTrue, reason: lines.join('\n'));
    expect(lines.any((l) => l.startsWith('[screen] opened')), isTrue, reason: lines.join('\n'));
  });
  testWidgets('backing out of the scanner leaves the other buttons working', (tester) async {
    await openMethod(tester, 'Scan code');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    // Nothing stays locked: Tap phones opens straight away.
    await go(tester, 'Tap phones');
    expect(find.text('Hold the backs of the phones together'), findsOneWidget);
  });
  testWidgets('a switch turned on outside the app opens the page by itself', (tester) async {
    engine.missing = const [Need.wifi];
    await openMethod(tester, 'Tap phones');
    expect(find.text('Turn these on to continue'), findsOneWidget);
    expect(engine.tapped.hasListener, isFalse);

    // Switched on from the quick panel: no tap in the app, no return to the app.
    engine.missing = const [];
    await tester.pump(const Duration(seconds: 3));
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Turn these on to continue'), findsNothing);
    expect(engine.tapped.hasListener, isTrue);
  });
  testWidgets('NFC off closes the tap page until it is on', (tester) async {
    engine.missing = const [Need.nfc];
    await openMethod(tester, 'Tap phones');
    await tester.pump();
    expect(find.text('Hold the backs of the phones together'), findsNothing);
    expect(find.text('NFC is off. Turn it on so phones can connect by tapping.'), findsOneWidget);

    engine.missing = const [];
    await tester.tap(find.text('Turn on'));
    await tester.pump();
    expect(engine.fixed, [Need.nfc]);
    expect(find.text('Hold the backs of the phones together'), findsOneWidget);
  });

  testWidgets('Receive does not start while Wi-Fi is off', (tester) async {
    engine.missing = const [Need.wifi];
    await open(tester);
    await go(tester, 'Receive');
    await tester.pump();
    expect(find.text('Wi-Fi is off. Nearvolt needs it to send and receive.'), findsOneWidget);
    expect(engine.receiveSessions, 0);

    engine.missing = const [];
    await tester.tap(find.text('Turn on'));
    await tester.pump();
    expect(engine.receiveSessions, 1);
  });

  testWidgets('the camera does not open while Wi-Fi is off', (tester) async {
    engine.missing = const [Need.wifi];
    await openMethod(tester, 'Scan code');
    await tester.pump();
    expect(find.text('Wi-Fi is off. Nearvolt needs it to send and receive.'), findsOneWidget);
    expect(engine.scans, 0);

    await go(tester, 'Not now');
    expect(engine.scans, 0);

    await go(tester, 'Scan code');
    engine.missing = const [];
    await tester.tap(find.text('Turn on'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(engine.scans, 1);
  });

  testWidgets('files chosen first are sent once the connection is set up, not before', (tester) async {
    await open(tester);
    await go(tester, 'Send');
    await go(tester, 'Choose files');
    await go(tester, 'Files');
    expect(find.text('4 files · 821 MB'), findsOneWidget);

    await go(tester, 'Tap phones');
    engine.tapped.add('NV1TAP');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.queued, isEmpty, reason: 'still connecting');

    engine.sending!.add(const Connection(phase: LinkPhase.waiting, peer: 'Tapped phone'));
    await tester.pump();
    expect(engine.queued, isEmpty, reason: 'not accepted yet');
    // With no code to compare, the screen still says what is being waited for.
    expect(find.text('Waiting for Tapped phone to accept on their phone.'), findsOneWidget);

    engine.sending!.add(const Connection(phase: LinkPhase.connected, peer: 'Tapped phone'));
    await tester.pump();
    expect(engine.queued, [album]);

    engine.sending!.add(const Connection(phase: LinkPhase.reconnecting, peer: 'Tapped phone'));
    engine.sending!.add(const Connection(phase: LinkPhase.connected, peer: 'Tapped phone'));
    await tester.pump();
    expect(engine.queued, [album], reason: 'sent once, not again after a reconnect');
  });

  testWidgets('until the other phone accepts there is Cancel, not Disconnect, and no Add files', (tester) async {
    await openMethod(tester, 'Tap phones');
    engine.tapped.add(rahul.ticket!);
    await tester.pumpAndSettle();

    expect(find.text('Connecting…'), findsOneWidget);
    expect(find.text('Cancel'), findsOneWidget);
    expect(find.text('Disconnect'), findsNothing);
    expect(find.text('Add files'), findsNothing);

    engine.sending!.add(const Connection(phase: LinkPhase.waiting, peer: rahulName, code: '4821'));
    await tester.pumpAndSettle();
    expect(find.text('Cancel'), findsOneWidget);
    expect(find.text('Add files'), findsNothing);

    engine.sending!.add(connected);
    await tester.pumpAndSettle();
    expect(find.text('Disconnect'), findsOneWidget);
    expect(find.text('Add files'), findsOneWidget);

    engine.sending!.add(connected.copyWith(phase: LinkPhase.reconnecting));
    await tester.pumpAndSettle();
    expect(find.text('Stop reconnecting'), findsOneWidget);
    expect(find.text('Disconnect'), findsNothing);
    expect(find.text('Add files'), findsNothing);

    // Cancelling before the connection is set up needs no confirmation.
    await tester.tap(find.text('Stop reconnecting'));
    await tester.pumpAndSettle();
    expect(engine.disconnects, 1);
    expect(find.text('Disconnect from $rahulName?'), findsNothing);
  });

  testWidgets('the question closes by itself when the phone asking leaves, and nothing is answered', (tester) async {
    await open(tester);
    await go(tester, 'Receive');
    const asking = Connection(
      ticket: 'NV1ABC',
      phase: LinkPhase.waiting,
      peer: rahulName,
      request: Request(id: 3, from: rahulName, code: '4821'),
    );
    engine.receiving!.add(asking);
    await tester.pumpAndSettle();
    expect(find.text('wants to connect and send you files'), findsOneWidget);

    // The engine says the phone is gone: the request is cleared.
    engine.receiving!.add(const Connection(ticket: 'NV1ABC'));
    await tester.pumpAndSettle();

    expect(find.text('wants to connect and send you files'), findsNothing);
    expect(find.text('$rahulName cancelled.'), findsOneWidget);
    expect(engine.answered, isNull);
  });

  testWidgets('accepts one connection, receives every batch, then opens Receive again', (tester) async {
    await open(tester);
    await go(tester, 'Receive');
    expect(find.text('Visible as Pixel 8'), findsOneWidget);
    expect(engine.receiveSessions, 1);

    const waiting = Connection(ticket: 'NV1ABC');
    engine.receiving!.add(waiting);
    await tester.pumpAndSettle();
    expect(find.bySemanticsLabel('QR code to connect'), findsOneWidget);

    engine.receiving!.add(
      waiting.copyWith(
        phase: LinkPhase.waiting,
        peer: rahulName,
        request: const Request(id: 7, from: rahulName, code: '4821'),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('wants to connect and send you files'), findsOneWidget);
    expect(find.text('4 8 2 1'), findsOneWidget);

    await tester.tap(find.text('Accept'));
    await tester.pumpAndSettle();
    expect(engine.answered, isTrue);
    expect(engine.answeredId, 7, reason: 'the answer names the question the user saw');

    engine.receiving!.add(connected);
    await tester.pumpAndSettle();
    expect(find.text('Receiving from'), findsOneWidget);
    expect(find.text('Files $rahulName sends appear here.'), findsOneWidget);

    engine.receiving!.add(
      connected.copyWith(
        batches: const [
          Batch(id: 0, files: 4, bytes: 821400000, state: BatchState.done),
          Batch(id: 1, files: 1, bytes: 2000000, state: BatchState.done),
        ],
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Received and verified'), findsNWidgets(2));

    engine.receiving!.add(connected.copyWith(phase: LinkPhase.ended, endedByPeer: true));
    await tester.pumpAndSettle();
    expect(find.text('$rahulName disconnected'), findsOneWidget);

    await tester.tap(find.text('Done'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Visible as Pixel 8'), findsOneWidget);
    expect(engine.receiveSessions, 2);
  });
}
