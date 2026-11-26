import 'package:material_ui/material_ui.dart';

import 'engine/engine.dart';
import 'pages/send_page.dart';
import 'theme.dart';

class NearvoltApp extends StatelessWidget {
  const NearvoltApp({super.key, required this.engine});

  final TransferEngine engine;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Nearvolt',
      debugShowCheckedModeBanner: false,
      theme: buildTheme(),
      home: SendPage(engine: engine),
    );
  }
}
