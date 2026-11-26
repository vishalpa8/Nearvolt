import 'package:material_ui/material_ui.dart';

import 'engine/engine.dart';
import 'engine/models.dart';
import 'pages/send_page.dart';
import 'theme.dart';

class NearvoltApp extends StatelessWidget {
  const NearvoltApp({super.key, required this.engine, required this.profile});

  final TransferEngine engine;
  final DeviceProfile profile;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Nearvolt',
      debugShowCheckedModeBanner: false,
      theme: buildTheme(),
      home: SendPage(engine: engine, profile: profile),
    );
  }
}
