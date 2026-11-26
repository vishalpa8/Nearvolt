import 'package:flutter_test/flutter_test.dart';
import 'package:proxima/engine/demo_engine.dart';
import 'package:proxima/main.dart';

void main() {
  testWidgets('home finds nearby devices', (tester) async {
    await tester.pumpWidget(ProximaApp(engine: DemoEngine()));
    expect(find.text('Send to anyone nearby'), findsOneWidget);
    expect(find.text('Looking for nearby devices…'), findsOneWidget);

    await tester.pump(const Duration(seconds: 4));
    expect(find.text("Rahul's Pixel 9"), findsOneWidget);
    expect(find.text('Studio laptop'), findsOneWidget);
    expect(find.text('Choose files'), findsOneWidget);
  });

  testWidgets('sending runs through reconnect to verified', (tester) async {
    await tester.pumpWidget(ProximaApp(engine: DemoEngine()));
    await tester.pump(const Duration(seconds: 2));

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
