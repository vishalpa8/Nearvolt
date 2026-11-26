import 'package:material_ui/material_ui.dart';

import 'engine/engine.dart';
import 'engine/models.dart';
import 'pages/home_page.dart';
import 'theme.dart';
import 'ui/templates.dart';

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
      home: HomePage(engine: engine, profile: profile),
    );
  }
}

/// Shown instead of the app when it can't start, so users know what to share.
class StartupFailure extends StatelessWidget {
  const StartupFailure({super.key, required this.reason});

  final String reason;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Nearvolt',
      debugShowCheckedModeBanner: false,
      theme: buildTheme(),
      home: Builder(
        builder: (context) {
          final text = Theme.of(context).textTheme;
          return PageScaffold(
            title: 'Nearvolt',
            body: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                const Icon(Icons.error_outline_rounded, size: 56, color: AppColors.danger),
                const SizedBox(height: Space.l),
                Text("Nearvolt couldn't start", style: text.titleLarge, textAlign: TextAlign.center),
                const SizedBox(height: Space.s),
                Text(
                  'Installing the latest version from GitHub usually fixes this. If it doesn\'t, share '
                  'Download › Nearvolt › nearvolt.log.',
                  style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
                  textAlign: TextAlign.center,
                ),
                const SizedBox(height: Space.l),
                Text(reason, style: text.bodySmall, textAlign: TextAlign.center),
              ],
            ),
          );
        },
      ),
    );
  }
}
