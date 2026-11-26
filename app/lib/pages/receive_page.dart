import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/atoms.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'transfer_page.dart';

/// While this page is open, nearby senders can find this device, scan its code
/// or tap phones with it.
class ReceivePage extends StatefulWidget {
  const ReceivePage({super.key, required this.engine, required this.profile});

  final TransferEngine engine;
  final DeviceProfile profile;

  @override
  State<ReceivePage> createState() => _ReceivePageState();
}

class _ReceivePageState extends State<ReceivePage> {
  StreamSubscription<ReceiveEvent>? _session;
  StreamController<TransferProgress>? _transfer;
  String? _ticket;

  @override
  void initState() {
    super.initState();
    _open();
  }

  void _open() {
    _session = widget.engine.receive().listen(_onEvent);
  }

  Future<void> _onEvent(ReceiveEvent event) async {
    switch (event) {
      case ReceiveReady(:final ticket):
        setState(() => _ticket = ticket);
      case ReceiveOffer(:final offer):
        final accepted = await showReceivePrompt(context, offer);
        widget.engine.answer(accept: accepted);
        if (accepted && mounted) await _follow(offer);
      case ReceiveProgress(:final progress):
        _transfer?.add(progress);
    }
  }

  /// Shows the transfer, then opens Receive again for the next one.
  Future<void> _follow(IncomingOffer offer) async {
    final transfer = _transfer = StreamController<TransferProgress>.broadcast();
    await Navigator.of(context).push(MaterialPageRoute<void>(
      builder: (_) => TransferPage(
        progress: transfer.stream,
        peerName: offer.from.name,
        incoming: true,
        onStop: widget.engine.stopTransfer,
      ),
    ));
    await transfer.close();
    _transfer = null;
    await _session?.cancel();
    if (!mounted) return;
    setState(() => _ticket = null);
    _open();
  }

  @override
  void dispose() {
    _session?.cancel();
    _transfer?.close();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final ticket = _ticket;
    return PageScaffold(
      title: 'Receive',
      onBack: () => Navigator.of(context).maybePop(),
      body: SingleChildScrollView(
        child: Column(
          children: [
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
            _Hint(icon: Icons.qr_code_scanner_rounded, text: 'Scan this code on the other device'),
            if (widget.profile.hasNfc) _Hint(icon: Icons.nfc_rounded, text: 'Or tap the phones together'),
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
