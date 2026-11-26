import 'package:material_ui/material_ui.dart';

import '../theme.dart';

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
              child: Padding(padding: const EdgeInsets.symmetric(horizontal: Space.xl), child: body),
            ),
            if (bottom != null)
              Padding(padding: const EdgeInsets.fromLTRB(Space.xl, Space.s, Space.xl, Space.xl), child: bottom),
          ],
        ),
      ),
    );
  }
}
