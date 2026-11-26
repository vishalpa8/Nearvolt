import 'package:material_ui/material_ui.dart';

import 'app.dart';
import 'engine/native_engine.dart';
import 'engine/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final engine = NativeEngine();
  final profile = await engine.start();
  runApp(NearvoltApp(engine: engine, profile: profile));
}
