import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/molecules.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'receive_page.dart';
import 'send_page.dart';

/// Send or Receive. Nothing looks for other phones until one is chosen.
class HomePage extends StatefulWidget {
  const HomePage({super.key, required this.engine, required this.profile});

  final TransferEngine engine;
  final DeviceProfile profile;

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  String? _update;

  @override
  void initState() {
    super.initState();
    widget.engine.checkForUpdate().then((version) {
      if (mounted) setState(() => _update = version);
    });
  }

  void _open(Widget page) => Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => page));

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final (engine, profile) = (widget.engine, widget.profile);
    return PageScaffold(
      title: 'Nearvolt',
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (_update case final version?) ...[
            const SizedBox(height: Space.l),
            UpdateBanner(version: version, install: engine.installUpdate),
          ],
          const Spacer(),
          Text('Move files to anyone nearby', style: text.displaySmall),
          const SizedBox(height: Space.s),
          Text(
            'No internet or account needed. Every file arrives exactly as it was sent.',
            style: text.bodyLarge?.copyWith(color: AppColors.textSecondary),
          ),
          const Spacer(flex: 2),
        ],
      ),
      bottom: IntrinsicHeight(
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: ActionCard(
                icon: Icons.north_east_rounded,
                title: 'Send',
                detail: 'Pick files and a phone',
                primary: true,
                onTap: () => _open(SendPage(engine: engine, profile: profile)),
              ),
            ),
            const SizedBox(width: Space.m),
            Expanded(
              child: ActionCard(
                icon: Icons.south_west_rounded,
                title: 'Receive',
                detail: 'Get files from a phone',
                onTap: () => _open(ReceivePage(engine: engine, profile: profile)),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
