import 'package:flutter_test/flutter_test.dart';
import 'package:nearvolt/app.dart';
import 'package:nearvolt/engine/demo_engine.dart';

void main() {
  testWidgets('start page finds devices with Receive open', (tester) async {
    await tester.pumpWidget(NearvoltApp(engine: DemoEngine()));
    expect(find.text('Send to anyone nearby'), findsOneWidget);
    expect(find.text('Looking for devices with Receive open…'), findsOneWidget);

    await tester.pump(const Duration(seconds: 4));
    expect(find.text("Rahul's Pixel 9"), findsOneWidget);
    expect(find.text('Studio laptop'), findsOneWidget);
    expect(find.text('Choose files'), findsOneWidget);
  });

  testWidgets('sending runs through reconnect to verified', (tester) async {
    await tester.pumpWidget(NearvoltApp(engine: DemoEngine()));
    await tester.pump(const Duration(seconds: 4));

    await tester.tap(find.text("Rahul's Pixel 9"));
    await tester.pumpAndSettle();
    expect(find.text('Ready to send'), findsOneWidget);
    expect(find.text('1,903 files'), findsOneWidget);

    await tester.tap(find.text("Send to Rahul's Pixel 9"));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Sending to'), findsOneWidget);

    var sawReconnect = false;
    for (var i = 0; i < 300 && find.text('All 1,903 files verified').evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 100));
      sawReconnect |= find.text('Link dropped · Reconnecting…').evaluate().isNotEmpty;
    }
    expect(sawReconnect, isTrue);
    expect(find.text('All 1,903 files verified'), findsOneWidget);
  });
}
