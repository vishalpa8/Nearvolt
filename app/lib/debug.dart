import 'package:flutter/rendering.dart';
import 'package:material_ui/material_ui.dart';

/// Detailed logging while Nearvolt is still in development: every screen, tap, state
/// change, message, call into Android and engine event goes to the shared log, so a
/// problem can be traced afterwards. Set to false before a public launch (the log then
/// keeps only milestones and errors).
const detailedLogging = true;

/// Where log lines go. Set once at start-up; without it (in tests) nothing is written.
class DebugLog {
  static void Function(String tag, String message)? sink;

  static void log(String tag, String message) {
    if (detailedLogging) sink?.call(tag, message);
  }
}

/// Writes every screen, dialog and sheet that opens or closes.
class LogObserver extends NavigatorObserver {
  String _name(Route<dynamic>? route) => route == null ? 'none' : (route.settings.name ?? route.runtimeType.toString());

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      DebugLog.log('screen', 'opened ${_name(route)}, over ${_name(previousRoute)}');

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      DebugLog.log('screen', 'closed ${_name(route)}, back to ${_name(previousRoute)}');

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) =>
      DebugLog.log('screen', 'replaced ${_name(oldRoute)} with ${_name(newRoute)}');

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      DebugLog.log('screen', 'removed ${_name(route)}');
}

/// Wraps the whole app. Logs what the user taps, by the label a screen reader would
/// speak, and when the app goes to the background or comes back.
class DebugLayer extends StatefulWidget {
  const DebugLayer({super.key, required this.child});

  final Widget child;

  @override
  State<DebugLayer> createState() => _DebugLayerState();
}

class _DebugLayerState extends State<DebugLayer> {
  SemanticsHandle? _semantics;
  AppLifecycleListener? _lifecycle;

  @override
  void initState() {
    super.initState();
    if (!detailedLogging) return;
    // Tap labels come from the accessibility tree, which is only built when asked for.
    _semantics = SemanticsBinding.instance.ensureSemantics();
    _lifecycle = AppLifecycleListener(onStateChange: (state) => DebugLog.log('app', 'now ${state.name}'));
  }

  @override
  void dispose() {
    _lifecycle?.dispose();
    _semantics?.dispose();
    super.dispose();
  }

  void _tapped(PointerUpEvent event) {
    try {
      final label = labelAt(event.position, View.of(context).devicePixelRatio);
      DebugLog.log('tap', label == null ? 'on nothing tappable at ${_point(event.position)}' : '"$label"');
    } on Object catch (e) {
      DebugLog.log('tap', 'could not tell what was tapped: $e');
    }
  }

  static String _point(Offset p) => '${p.dx.round()},${p.dy.round()}';

  @override
  Widget build(BuildContext context) {
    if (!detailedLogging) return widget.child;
    return Listener(behavior: HitTestBehavior.translucent, onPointerUp: _tapped, child: widget.child);
  }
}

/// The label of the tappable thing under [position] (logical pixels), or null.
String? labelAt(Offset position, double pixelRatio) {
  final owner = RendererBinding.instance.renderViews.firstOrNull?.owner?.semanticsOwner;
  final root = owner?.rootSemanticsNode;
  if (root == null) return null;
  // The tree is laid out in physical pixels.
  return _find(root, position * pixelRatio);
}

String? _find(SemanticsNode node, Offset pointInParent) {
  var point = pointInParent;
  final transform = node.transform;
  if (transform != null) {
    final inverse = Matrix4.tryInvert(transform);
    if (inverse == null) return null;
    point = MatrixUtils.transformPoint(inverse, pointInParent);
  }
  if (!node.rect.contains(point)) return null;
  String? deepest;
  node.visitChildren((child) {
    deepest ??= _find(child, point);
    return deepest == null;
  });
  if (deepest != null) return deepest;
  final data = node.getSemanticsData();
  if (!data.hasAction(SemanticsAction.tap) && !data.hasAction(SemanticsAction.longPress)) return null;
  final text = data.label.isNotEmpty ? data.label : data.tooltip;
  return text.isEmpty ? 'a button without a label' : text.replaceAll('\n', ' ');
}
