import 'package:flutter/foundation.dart' show listEquals;
import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme.dart';
import 'organisms.dart';

/// The layout every page shares: an optional back row, scrollable content and a
/// bottom action that stays in reach of the thumb.
class PageScaffold extends StatelessWidget {
  const PageScaffold({super.key, required this.body, this.title, this.trailing, this.bottom, this.onBack});

  final Widget body;
  final String? title;
  final Widget? trailing;
  final Widget? bottom;

  /// Shows a back button when set.
  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context) {
    final header = title != null || trailing != null || onBack != null;
    return Scaffold(
      body: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (header)
              Padding(
                padding: EdgeInsets.fromLTRB(onBack == null ? Space.xl : Space.s, Space.s, Space.xl, 0),
                child: SizedBox(
                  height: 48,
                  child: Row(
                    children: [
                      if (onBack != null)
                        IconButton(onPressed: onBack, tooltip: 'Back', icon: const Icon(Icons.arrow_back_rounded)),
                      if (title != null)
                        Expanded(child: Text(title!, style: Theme.of(context).textTheme.titleMedium))
                      else
                        const Spacer(),
                      ?trailing,
                    ],
                  ),
                ),
              ),
            Expanded(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: Space.xl),
                child: body,
              ),
            ),
            if (bottom != null)
              Padding(padding: const EdgeInsets.fromLTRB(Space.xl, Space.s, Space.xl, Space.xl), child: bottom),
          ],
        ),
      ),
    );
  }
}

/// For pages that reach other phones. When the page opens, asks Android for what
/// [purpose] uses, then calls [restart]; keeps what's still missing in [needs],
/// and calls [restart] again whenever that changes, including after the user
/// flips a switch outside the app.
mixin NeedsMixin<T extends StatefulWidget> on State<T> {
  List<Need> needs = const [];
  late final AppLifecycleListener _lifecycle;
  var _busy = false;

  TransferEngine get engine;

  Purpose get purpose;

  /// Starts, or starts again, with whatever is available now.
  void restart();

  @override
  void initState() {
    super.initState();
    _lifecycle = AppLifecycleListener(onResume: _recheck);
    _update(engine.prepare(purpose), always: true);
  }

  @override
  void dispose() {
    _lifecycle.dispose();
    super.dispose();
  }

  /// The notice for the most important missing thing, if any.
  Widget? get needNotice =>
      needs.isEmpty ? null : NeedNotice(need: needs.first, onFix: () => _update(_fix(needs.first)));

  Future<List<Need>> _fix(Need need) async {
    await engine.fix(need);
    return engine.needs(purpose);
  }

  /// Only while this page is on top, so an open connection is never restarted.
  void _recheck() {
    if (!_busy && (ModalRoute.of(context)?.isCurrent ?? true)) _update(engine.needs(purpose));
  }

  Future<void> _update(Future<List<Need>> next, {bool always = false}) async {
    _busy = true;
    final now = await next;
    _busy = false;
    if (!mounted || (!always && listEquals(now, needs))) return;
    setState(() => needs = now);
    restart();
  }
}
