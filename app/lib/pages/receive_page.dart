import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/atoms.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'connection_page.dart';

/// While this page is open, a sender can scan its code, tap phones with it,
/// pick it from their Nearby list or plug in a cable. Once one connects, it
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
  Connection _c = const Connection();
  var _asking = false;
  var _showing = false;

  @override
  TransferEngine get engine => widget.engine;

  @override
  Purpose get purpose => Purpose.receive;

  /// A new session, so the code and what's advertised match what's on now.
  @override
  Future<void> restart() async {
    await _session?.cancel();
    if (!mounted) return;
    setState(() => _c = const Connection());
    _session = engine.receive().listen(_update);
  }

  Future<void> _update(Connection c) async {
    setState(() => _c = c);
    _updates.add(c);
    if (c.request case final request? when !_asking && c.phase == LinkPhase.waiting) {
      _asking = true;
      final accepted = await showReceivePrompt(context, request);
      _asking = false;
      engine.answer(accept: accepted);
    }
    if (c.phase == LinkPhase.connected && !_showing && mounted) {
      _showing = true;
      await Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => ConnectionPage(engine: engine, updates: _updates.stream, initial: _c, sending: false),
        ),
      );
      _showing = false;
      if (mounted) await restart();
    }
  }

  @override
  void dispose() {
    _session?.cancel();
    _updates.close();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final ticket = _c.ticket;
    return PageScaffold(
      title: 'Receive',
      onBack: () => Navigator.of(context).maybePop(),
      body: SingleChildScrollView(
        child: Column(
          children: [
            if (needNotice case final notice?) ...[const SizedBox(height: Space.l), notice],
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
            _Hint(icon: Icons.radar_rounded, text: 'Or pick ${widget.profile.name} in their Nearby list'),
            const _Hint(icon: Icons.usb_rounded, text: 'Or connect the phones with a USB cable'),
          ],
        ),
      ),
    );
  }
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
