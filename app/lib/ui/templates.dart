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
/// [purpose] uses; while anything is still off, the page stays closed behind a
/// [NeedsGate] and nothing it would start is started. Once everything is on,
/// calls [restart]; it checks again whenever the app comes back to the front, so
/// switches flipped outside the app count, and a switch turned off later closes
/// the page again.
mixin NeedsMixin<T extends StatefulWidget> on State<T> {
  List<Need> needs = const [];
  late final AppLifecycleListener _lifecycle;
  var _busy = false;
  var _checked = false;

  TransferEngine get engine;

  Purpose get purpose;

  /// Starts, or starts again, now that everything it needs is on.
  void restart();

  /// Stops what the page started, because something it needs went off.
  void pause() {}

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

  /// The page to show instead of the real one while it can't be used yet, or null
  /// when it can. Until the first check is done it is just empty, so a page never
  /// flashes open and then closes.
  Widget? blockedPage(String title) {
    if (_checked && needs.isEmpty) return null;
    return PageScaffold(
      title: title,
      onBack: () => Navigator.of(context).maybePop(),
      body: _checked ? NeedsGate(needs: needs, onFix: (need) => _update(_fix(need))) : const SizedBox.shrink(),
    );
  }

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
    if (!mounted || (_checked && !always && listEquals(now, needs))) return;
    setState(() {
      needs = now;
      _checked = true;
    });
    if (now.isEmpty) {
      restart();
    } else {
      pause();
    }
  }
}
