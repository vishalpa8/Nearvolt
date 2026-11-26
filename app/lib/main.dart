import 'package:material_ui/material_ui.dart';

import 'engine/demo_engine.dart';
import 'engine/engine.dart';
import 'engine/models.dart';
import 'screens/home_screen.dart';
import 'screens/pick_screen.dart';
import 'screens/transfer_screen.dart';
import 'theme/app_theme.dart';

void main() {
  runApp(NearvoltApp(engine: DemoEngine()));
}

class NearvoltApp extends StatelessWidget {
  const NearvoltApp({super.key, required this.engine});

  final TransferEngine engine;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Nearvolt',
      debugShowCheckedModeBanner: false,
      theme: buildTheme(),
      onGenerateRoute: (settings) => MaterialPageRoute<void>(
        settings: settings,
        builder: (_) => _screen(settings.name),
      ),
    );
  }

  /// `/nearby`, `/pick`, `/receive`, `/transfer` and `/done` open a screen directly
  /// with sample data, for design review (on the web: `#/transfer`).
  Widget _screen(String? route) {
    const peer = DemoEngine.sampleOffer;
    return switch (route) {
      '/nearby' => HomeScreen(engine: DemoEngine(discoveryDelay: Duration.zero)),
      '/pick' => PickScreen(
          engine: engine,
          items: DemoEngine.sampleItems,
          devices: DemoEngine.devices,
          initial: DemoEngine.devices.first,
        ),
      '/receive' => HomeScreen(engine: DemoEngine(discoveryDelay: Duration.zero), showSampleOffer: true),
      '/transfer' => TransferScreen(
          progress: Stream.value(const TransferProgress(
            phase: TransferPhase.transferring,
            doneBytes: 193440000000,
            totalBytes: 312000000000,
            doneFiles: 154,
            totalFiles: 248,
            bytesPerSecond: 1120000000,
          )),
          peer: peer.from,
          incoming: false,
        ),
      '/done' => TransferScreen(
          progress: Stream.value(const TransferProgress(
            phase: TransferPhase.completed,
            doneBytes: 312000000000,
            totalBytes: 312000000000,
            doneFiles: 248,
            totalFiles: 248,
          )),
          peer: peer.from,
          incoming: false,
        ),
      _ => HomeScreen(engine: engine),
    };
  }
}
