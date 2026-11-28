import 'dart:ui' show AppLifecycleState, Size;

import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart'
    show CircularProgressIndicator, LinearProgressIndicator, NavigatorState, Navigator;
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

  testWidgets('nothing looks for phones until a way to connect is chosen', (tester) async {
    await open(tester);
    expect(find.text('Move files to anyone nearby'), findsOneWidget);

    await go(tester, 'Send');
    expect(find.text('How do you want to connect?'), findsOneWidget);
    for (final method in ['Tap phones', 'Scan code', 'Nearby phones', 'USB cable']) {
      expect(find.text(method), findsOneWidget);
    }
    expect(engine.prepared, isEmpty);
    expect(engine.discoveries, 0);
    expect(engine.receiveSessions, 0);

    await go(tester, 'Tap phones');
    expect(engine.prepared, [Purpose.tap]);
    expect(engine.tapped.hasListener, isTrue);
    expect(engine.discoveries, 0);
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
    await openMethod(tester, 'Nearby phones');
    expect(find.text('Looking for devices with Receive open…'), findsOneWidget);
    engine.devices.add([rahul, tablet]);
    await tester.pumpAndSettle();

    await tester.tap(find.text(rahulName));
    await tester.pumpAndSettle();
    expect(engine.connectedTo, rahul);
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.devices.hasListener, isFalse, reason: 'no searching while connected');

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
    expect(find.text('Nearby'), findsOneWidget);
    expect(engine.devices.hasListener, isTrue, reason: 'searching again once disconnected');
  });

  Future<void> connectByCode(WidgetTester tester) async {
    await open(tester);
    await go(tester, 'Send');
    await go(tester, 'Choose files');
    await go(tester, 'Files');
    await go(tester, 'Nearby phones');
    engine.devices.add([rahul]);
    await tester.pumpAndSettle();
    await tester.tap(find.text(rahulName));
    await tester.pumpAndSettle();
    engine.sending!.add(connected.copyWith(code: '4821'));
    await tester.pumpAndSettle();
  }

  testWidgets('a code the sender must compare holds back files and Add files until it matches', (tester) async {
    await connectByCode(tester);
    expect(find.text("Does $rahulName show this same code?"), findsOneWidget);
    expect(find.text('Connected · check the code'), findsOneWidget);
    expect(engine.queued, isEmpty);
    expect(find.text('Add files'), findsNothing);

    await tester.tap(find.text('It matches'));
    await tester.pumpAndSettle();
    expect(engine.queued, [album]);
    expect(find.text('Add files'), findsOneWidget);
    expect(find.text('It matches'), findsNothing);
  });

  testWidgets('a code that does not match disconnects and sends nothing', (tester) async {
    await connectByCode(tester);
    await tester.tap(find.text("It doesn't match"));
    await tester.pump();

    expect(engine.disconnects, 1);
    expect(engine.queued, isEmpty);
    expect(find.textContaining('this may not be $rahulName'), findsOneWidget);
  });

  testWidgets('Cancel before the other phone accepts leaves the screen at once', (tester) async {
    await openMethod(tester, 'Nearby phones');
    engine.devices.add([rahul]);
    await tester.pumpAndSettle();
    await tester.tap(find.text(rahulName));
    await tester.pumpAndSettle();
    expect(find.text('Connecting…'), findsOneWidget);

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(engine.disconnects, 1);
    expect(find.text('Connecting…'), findsNothing);
    expect(find.text('Nearby'), findsOneWidget, reason: 'back on the Nearby page, not stuck on "Cancelling…"');
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

  testWidgets('searching the network failing is said in words, not shown as an empty list', (tester) async {
    await openMethod(tester, 'Nearby phones');
    expect(find.text(searchFailedText), findsNothing);
    engine.searchFailed.value = true;
    await tester.pump();
    expect(find.text(searchFailedText), findsOneWidget);
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
    engine.receiving!.add(
      const Connection(ticket: 'NV1ABC', problems: [SetupProblem.otherNetworks, SetupProblem.notListed]),
    );
    await tester.pumpAndSettle();
    expect(find.text(setupProblemText(SetupProblem.otherNetworks)), findsOneWidget);
    expect(find.text(setupProblemText(SetupProblem.notListed)), findsOneWidget);
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

    engine.usb.add(const NearbyDevice(id: 'usb', name: 'USB cable', kind: DeviceKind.phone, link: 'USB cable'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Sending to'), findsOneWidget);
    expect(engine.connectedTo?.id, 'usb');
  });

  testWidgets('a missing permission closes the page: nothing is searched until it is allowed', (tester) async {
    engine.missing = const [Need.permission, Need.bluetooth];
    await openMethod(tester, 'Nearby phones');
    expect(find.text('Turn these on to continue'), findsOneWidget);
    expect(find.text('Allow Nearby devices so Nearvolt can find phones and be found.'), findsOneWidget);
    expect(find.text('Bluetooth is off. Nearvolt needs it so phones can find each other.'), findsOneWidget);
    expect(engine.discoveries, 0);

    engine.missing = const [];
    await tester.tap(find.text('Allow'));
    await tester.pump();
    expect(engine.fixed, [Need.permission]);
    expect(find.text('Turn these on to continue'), findsNothing);
    expect(engine.discoveries, 1);
  });

  testWidgets('a prompt that never answers does not leave the page blank for ever', (tester) async {
    engine.hangPrepare = true;
    await openMethod(tester, 'Nearby phones');
    expect(find.text('Turn these on to continue'), findsNothing);

    // Coming back to the app checks again, even though the first check never ended.
    engine.missing = const [Need.bluetooth];
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
    await go(tester, 'Nearby phones');

    expect(lines.where((l) => l.startsWith('[tap]')), isNotEmpty, reason: lines.join('\n'));
    expect(lines.any((l) => l.startsWith('[tap] "Send')), isTrue, reason: lines.join('\n'));
    expect(lines.any((l) => l.startsWith('[tap] "Nearby phones')), isTrue, reason: lines.join('\n'));
    expect(lines.any((l) => l.startsWith('[screen] opened')), isTrue, reason: lines.join('\n'));
  });
  testWidgets('backing out of the scanner leaves the other buttons working', (tester) async {
    await openMethod(tester, 'Scan code');
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    // Nothing stays locked: Nearby phones opens straight away.
    await go(tester, 'Nearby phones');
    expect(find.text('Nearby'), findsWidgets);
  });
  testWidgets('a switch turned on outside the app opens the page by itself', (tester) async {
    engine.missing = const [Need.wifi];
    await openMethod(tester, 'Nearby phones');
    expect(find.text('Turn these on to continue'), findsOneWidget);
    expect(engine.discoveries, 0);

    // Switched on from the quick panel: no tap in the app, no return to the app.
    engine.missing = const [];
    await tester.pump(const Duration(seconds: 3));
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Turn these on to continue'), findsNothing);
    expect(engine.discoveries, 1);
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

    engine.sending!.add(const Connection(phase: LinkPhase.connected, peer: 'Tapped phone'));
    await tester.pump();
    expect(engine.queued, [album]);

    engine.sending!.add(const Connection(phase: LinkPhase.reconnecting, peer: 'Tapped phone'));
    engine.sending!.add(const Connection(phase: LinkPhase.connected, peer: 'Tapped phone'));
    await tester.pump();
    expect(engine.queued, [album], reason: 'sent once, not again after a reconnect');
  });

  testWidgets('until the other phone accepts there is Cancel, not Disconnect, and no Add files', (tester) async {
    await openMethod(tester, 'Nearby phones');
    engine.devices.add([rahul]);
    await tester.pumpAndSettle();
    await tester.tap(find.text(rahulName));
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
