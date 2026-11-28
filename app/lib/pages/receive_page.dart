import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/messages.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/atoms.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'connection_page.dart';

/// While this page is open, a sender can scan its code, tap phones with it
/// or plug in a cable. Once one connects, it
/// stays connected until either phone disconnects.
class ReceivePage extends StatefulWidget {
  const ReceivePage({super.key, required this.engine, required this.profile});

  final TransferEngine engine;
  final DeviceProfile profile;

  @override
  State<ReceivePage> createState() => _ReceivePageState();
}

class _ReceivePageState extends State<ReceivePage> with NeedsMixin {
  StreamSubscription<Connection>? _session;
  final _updates = StreamController<Connection>.broadcast();
  final _withdrawn = StreamController<void>.broadcast();
  Connection _c = const Connection();
  var _asking = false;
  var _showing = false;

  /// Back was pressed. The page is on screen for a moment longer while it slides away,
  /// and a phone connecting in that moment must not open a connection screen over a
  /// Receive that is already closing.
  var _leaving = false;

  void _leave() {
    _leaving = true;
    _idle?.cancel();
    _session?.cancel();
    _session = null;
  }

  @override
  TransferEngine get engine => widget.engine;

  @override
  Purpose get purpose => Purpose.receive;

  /// How long Receive stays open with nobody connecting. A Wi-Fi Direct
  /// group and NFC cost battery.
  static const _idleLimit = Duration(minutes: 10);
  Timer? _idle;
  var _idleStopped = false;

  /// Something Receive needs went off: stop the Wi-Fi Direct group and listening until it is back.
  @override
  void pause() {
    _idle?.cancel();
    _session?.cancel();
    _session = null;
  }

  /// A new session, so the code matches what's on now.
  @override
  Future<void> restart() async {
    await _session?.cancel();
    if (!mounted) return;
    setState(() {
      _c = const Connection();
      _idleStopped = false;
    });
    _session = engine.receive().listen(_update);
    _armIdle();
  }

  void _armIdle() {
    _idle?.cancel();
    _idle = Timer(_idleLimit, _stopIdle);
  }

  /// Receive closes itself, unless someone is in the middle of connecting.
  Future<void> _stopIdle() async {
    if (!mounted) return;
    if (_asking || _showing) return _armIdle();
    await _session?.cancel();
    _session = null;
    if (mounted) setState(() => _idleStopped = true);
  }

  Future<void> _update(Connection c) async {
    setState(() => _c = c);
    _updates.add(c);
    // The phone that was asking left: the open question closes by itself.
    if (_asking && c.request == null) _withdrawn.add(null);
    if (c.request case final request? when !_asking && c.phase == LinkPhase.waiting) {
      _asking = true;
      final accepted = await showReceivePrompt(context, request, withdrawn: _withdrawn.stream);
      _asking = false;
      // No answer means it left first, and an answer nobody receives means the same.
      final delivered = accepted != null && engine.answer(request.id, accept: accepted);
      if (!delivered && mounted) {
        showNote(context, '${request.from} cancelled.');
      }
    }
    if (c.phase == LinkPhase.connected && !_showing && mounted && !_leaving) {
      _showing = true;
      _idle?.cancel();
      await Navigator.of(context).push(
        MaterialPageRoute<void>(
          settings: const RouteSettings(name: 'ConnectionPage (receiving)'),
          builder: (_) => ConnectionPage(engine: engine, updates: _updates.stream, initial: _c, sending: false),
        ),
      );
      _showing = false;
      if (mounted) await restart();
    }
  }

  @override
  void dispose() {
    _idle?.cancel();
    _session?.cancel();
    _updates.close();
    _withdrawn.close();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (blockedPage('Receive') case final page?) return page;
    final text = Theme.of(context).textTheme;
    final ticket = _c.ticket;
    return PopScope(
      onPopInvokedWithResult: (didPop, _) {
        if (didPop) _leave();
      },
      child: PageScaffold(
        title: 'Receive',
        onBack: () => Navigator.of(context).maybePop(),
        body: SingleChildScrollView(
          child: Column(children: [if (_idleStopped) ..._stopped(text) else ..._open(text, ticket)]),
        ),
      ),
    );
  }

  /// Receive closed itself after 10 quiet minutes.
  List<Widget> _stopped(TextTheme text) => [
    const SizedBox(height: Space.xxl),
    const Icon(Icons.battery_saver_rounded, size: 56, color: AppColors.textSecondary),
    const SizedBox(height: Space.l),
    Text(receiveIdleText, style: text.titleMedium, textAlign: TextAlign.center),
    const SizedBox(height: Space.xl),
    FilledButton(onPressed: restart, child: const Text('Open Receive again')),
  ];

  List<Widget> _open(TextTheme text, String? ticket) => [
    for (final problem in _c.problems) ...[
      const SizedBox(height: Space.l),
      Panel(
        child: Row(
          children: [
            const Icon(Icons.info_outline_rounded, color: AppColors.warning),
            const SizedBox(width: Space.m),
            Expanded(child: Text(setupProblemText(problem), style: text.bodyMedium)),
          ],
        ),
      ),
    ],
    const SizedBox(height: Space.xl),
    Panel(
      padding: const EdgeInsets.all(Space.l),
      child: SizedBox.square(
        dimension: 232,
        child: ticket == null
            ? const Center(child: CircularProgressIndicator(color: AppColors.accent))
            : QrView(data: ticket),
      ),
    ),
    const SizedBox(height: Space.xl),
    Text('Visible as ${widget.profile.name}', style: text.titleMedium, textAlign: TextAlign.center),
    const SizedBox(height: Space.s),
    const Pill(label: 'Waiting for a sender', color: AppColors.success),
    const SizedBox(height: Space.xxl),
    const _Hint(icon: Icons.qr_code_scanner_rounded, text: 'Scan this code on the other phone'),
    if (widget.profile.hasNfc) const _Hint(icon: Icons.nfc_rounded, text: 'Or tap the phones together'),
    const _Hint(icon: Icons.usb_rounded, text: 'Or connect the phones with a USB cable'),
  ];
}

class _Hint extends StatelessWidget {
  const _Hint({required this.icon, required this.text});

  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Space.s),
      child: Row(
        children: [
          Icon(icon, size: 20, color: AppColors.textSecondary),
          const SizedBox(width: Space.m),
          Expanded(child: Text(text, style: Theme.of(context).textTheme.bodyMedium)),
        ],
      ),
    );
  }
}
