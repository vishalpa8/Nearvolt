import 'dart:ui' show PlatformDispatcher;

import 'package:material_ui/material_ui.dart';

import 'app.dart';
import 'engine/native_engine.dart';
import 'engine/platform.g.dart';
import 'engine/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  // Every error, from the UI or the engine, lands in the log users can share.
  final platform = NearvoltPlatform();
  void report(Object error, StackTrace? stack) => platform.log('error', '$error\n$stack');
  FlutterError.onError = (details) {
    FlutterError.presentError(details);
    report(details.exception, details.stack);
  };
  PlatformDispatcher.instance.onError = (error, stack) {
    report(error, stack);
    return true;
  };

  try {
    await RustLib.init();
    final engine = NativeEngine();
    final profile = await engine.start();
    runApp(NearvoltApp(engine: engine, profile: profile));
  } on Object catch (error, stack) {
    report(error, stack);
    runApp(StartupFailure(reason: '$error'));
  }
}
