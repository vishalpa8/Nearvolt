import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/messages.dart';
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
  /// Counts update checks, so asking again starts a fresh one; null when none is shown.
  int? _check;

  /// An update check or install is running. Tapping again then does nothing, so two
  /// downloads never run at once.
  var _updating = false;

  /// A new version the background check found, waiting for the user's answer.
  String? _offered;

  /// The version the user said "Later" to; not offered again until a newer one comes out
  /// or the app is opened afresh.
  String? _later;

  Timer? _timer;

  /// How soon after opening the app, and then how often, it looks for a new version.
  static const firstCheck = Duration(seconds: 6);
  static const checkEvery = Duration(hours: 6);

  @override
  void initState() {
    super.initState();
    _timer = Timer(firstCheck, _backgroundCheck);
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  /// Looks quietly; a failure (no internet, GitHub busy) is not worth bothering anyone with.
  Future<void> _backgroundCheck() async {
    _timer = Timer(checkEvery, _backgroundCheck);
    // A manual check or install is already on screen, and one offer at a time is enough.
    if (_updating || _check != null || _offered != null) return;
    try {
      final version = await widget.engine.checkForUpdate();
      if (!mounted || _updating || _check != null) return;
      if (version != null && version != _later) setState(() => _offered = version);
    } on Object {
      // Try again at the next check.
    }
  }

  /// Whether the card running now should ask first; false after an offer the user accepted.
  var _ask = true;

  void _update({bool ask = true}) => setState(() {
    _offered = null;
    _ask = ask;
    _check = (_check ?? 0) + 1;
    _updating = true;
  });

  void _open(Widget page) => Navigator.of(context).push(
    MaterialPageRoute<void>(
      settings: RouteSettings(name: page.runtimeType.toString()),
      builder: (_) => page,
    ),
  );

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final (engine, profile) = (widget.engine, widget.profile);
    return PageScaffold(
      title: 'Nearvolt',
      trailing: _actions(context),
      // Scrolls when the phone is short or the text is large, and still spreads out when there is room.
      body: LayoutBuilder(
        builder: (context, box) => SingleChildScrollView(
          child: ConstrainedBox(
            constraints: BoxConstraints(minHeight: box.maxHeight),
            child: IntrinsicHeight(child: _intro(text, engine, profile)),
          ),
        ),
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

  /// Share and Check for updates, top right. With very large text the update button
  /// becomes an icon, so the row still fits a small phone.
  Widget _actions(BuildContext context) {
    final big = MediaQuery.textScalerOf(context).scale(10) > 13;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (big)
          IconButton(
            onPressed: _updating ? null : _update,
            tooltip: 'Check for updates',
            icon: const Icon(Icons.system_update_rounded, color: AppColors.accent),
          )
        else
          TextButton(onPressed: _updating ? null : _update, child: const Text('Check for updates')),
        // Anyone can be sent the link: it opens the phone's own share sheet.
        IconButton(
          onPressed: () => widget.engine.shareApp(shareText),
          tooltip: 'Share Nearvolt',
          icon: const Icon(Icons.send_rounded, color: AppColors.accent),
        ),
      ],
    );
  }

  Widget _intro(TextTheme text, TransferEngine engine, DeviceProfile profile) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (_offered case final version?) ...[
          const SizedBox(height: Space.l),
          UpdateOffer(
            version: version,
            onLater: () => setState(() {
              _later = version;
              _offered = null;
            }),
            onUpdate: () => _update(ask: false),
          ),
        ],
        if (_check case final check?) ...[
          const SizedBox(height: Space.l),
          UpdateCard(
            key: ValueKey(check),
            ask: _ask,
            current: profile.version,
            check: engine.checkForUpdate,
            install: engine.installUpdate,
            onClose: () => setState(() => _check = null),
            onDone: () {
              if (mounted) setState(() => _updating = false);
            },
          ),
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
    );
  }
}
