import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/atoms.dart';
import '../ui/molecules.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'transfer_page.dart';

/// Start page: choose what to send, then who to send it to.
class SendPage extends StatefulWidget {
  const SendPage({super.key, required this.engine});

  final TransferEngine engine;

  @override
  State<SendPage> createState() => _SendPageState();
}

class _SendPageState extends State<SendPage> {
  List<NearbyDevice> _devices = const [];
  final List<PickedItem> _items = [];
  NearbyDevice? _target;
  late final StreamSubscription<List<NearbyDevice>> _deviceSub;
  late final StreamSubscription<IncomingOffer> _offerSub;

  @override
  void initState() {
    super.initState();
    _deviceSub = widget.engine.nearbyDevices().listen((devices) => setState(() => _devices = devices));
    _offerSub = widget.engine.incomingOffers().listen(_onOffer);
  }

  @override
  void dispose() {
    _deviceSub.cancel();
    _offerSub.cancel();
    super.dispose();
  }

  Future<void> _pick([NearbyDevice? target]) async {
    final picked = await widget.engine.pickFiles();
    if (picked.isEmpty || !mounted) return;
    setState(() {
      _items.addAll(picked);
      _target = target ?? _target;
    });
  }

  void _send() {
    final target = _target;
    if (target == null || _items.isEmpty) return;
    final progress = widget.engine.send(target, List.of(_items));
    setState(_items.clear);
    Navigator.of(context).push(MaterialPageRoute<void>(
      builder: (_) => TransferPage(progress: progress, peer: target, incoming: false),
    ));
  }

  Future<void> _onOffer(IncomingOffer offer) async {
    final accepted = await showReceivePrompt(context, offer);
    if (!mounted) return;
    if (!accepted) return widget.engine.decline(offer);
    await Navigator.of(context).push(MaterialPageRoute<void>(
      builder: (_) => TransferPage(progress: widget.engine.receive(offer), peer: offer.from, incoming: true),
    ));
  }

  @override
  Widget build(BuildContext context) => _items.isEmpty ? _start(context) : _ready(context);

  Widget _start(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return PageScaffold(
      title: 'Nearvolt',
      trailing: widget.engine.isSimulated ? const Pill(label: 'Preview', style: PillStyle.tag) : null,
      body: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const SizedBox(height: Space.xxl),
            Text('Send to anyone nearby', style: text.displaySmall),
            const SizedBox(height: Space.s),
            Text(
              'No internet or account needed. Every file arrives exactly as it was sent.',
              style: text.bodyLarge?.copyWith(color: AppColors.textSecondary),
            ),
            const SizedBox(height: Space.xxl),
            Text('Nearby', style: text.titleMedium),
            const SizedBox(height: Space.m),
            DeviceList(devices: _devices, onTap: _pick),
          ],
        ),
      ),
      bottom: FilledButton.icon(
        onPressed: _pick,
        icon: const Icon(Icons.add_rounded),
        label: const Text('Choose files'),
      ),
    );
  }

  Widget _ready(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final target = _target;
    final bytes = _items.fold(0, (sum, item) => sum + item.bytes);
    final files = _items.fold(0, (sum, item) => sum + item.files);
    return PageScaffold(
      title: 'Ready to send',
      onBack: () => setState(_items.clear),
      body: ListView(
        padding: const EdgeInsets.only(top: Space.l, bottom: Space.xl),
        children: [
          SummaryCard(files: files, bytes: bytes),
          const SizedBox(height: Space.xxl),
          Text('Send to', style: text.titleMedium),
          const SizedBox(height: Space.s),
          DeviceList(devices: _devices, selectedId: target?.id, onTap: (d) => setState(() => _target = d)),
          const SizedBox(height: Space.xxl),
          Row(
            children: [
              Text('Files', style: text.titleMedium),
              const Spacer(),
              TextButton.icon(
                onPressed: _pick,
                icon: const Icon(Icons.add_rounded, size: 20),
                label: const Text('Add more'),
              ),
            ],
          ),
          for (final item in _items) FileRow(item: item, onRemove: () => setState(() => _items.remove(item))),
        ],
      ),
      bottom: FilledButton(
        onPressed: target == null ? null : _send,
        child: Text(target == null ? 'Choose a device' : 'Send to ${target.name}'),
      ),
    );
  }
}
