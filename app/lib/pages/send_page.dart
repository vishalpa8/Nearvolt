import 'dart:async';

import 'package:flutter/services.dart' show PlatformException;
import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/molecules.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'transfer_page.dart';

/// Finds phones with Receive open while it's visible. Choose what to send and
/// who to send it to, in either order.
class SendPage extends StatefulWidget {
  const SendPage({super.key, required this.engine, required this.profile});

  final TransferEngine engine;
  final DeviceProfile profile;

  @override
  State<SendPage> createState() => _SendPageState();
}

class _SendPageState extends State<SendPage> with NeedsMixin {
  List<NearbyDevice> _devices = const [];
  Selection? _selection;
  NearbyDevice? _target;
  StreamSubscription<List<NearbyDevice>>? _nearby;
  StreamSubscription<String>? _taps;

  @override
  TransferEngine get engine => widget.engine;

  @override
  void restart() {
    _pause();
    _listen();
  }

  void _listen() {
    _nearby = engine.nearby().listen((devices) => setState(() => _devices = devices));
    _taps = engine.taps().listen((ticket) => _connect(engine.deviceFor(ticket)));
  }

  void _pause() {
    _nearby?.cancel();
    _taps?.cancel();
  }

  @override
  void dispose() {
    _pause();
    super.dispose();
  }

  Future<void> _pick() async {
    final kind = await showPickMenu(context);
    if (kind == null) return;
    final selection = kind == PickKind.folder ? await engine.pickFolder() : await engine.pickFiles();
    if (selection != null && mounted) setState(() => _selection = selection);
  }

  /// Choosing a receiver before anything is picked opens the picker first.
  Future<void> _choose(NearbyDevice device) async {
    setState(() => _target = device);
    if (_selection == null) await _pick();
  }

  /// A scanned or tapped code already proves who the receiver is, so sending
  /// starts right away once files are chosen.
  Future<void> _connect(NearbyDevice device) async {
    await _choose(device);
    if (_selection != null && mounted) await _send();
  }

  Future<void> _scan() async {
    final String? ticket;
    try {
      ticket = await engine.scanCode();
    } on PlatformException catch (e) {
      return _tell(e.message ?? "Couldn't open the code scanner.");
    }
    if (ticket == null || !mounted) return;
    final NearbyDevice device;
    try {
      device = engine.deviceFor(ticket);
    } on Object {
      return _tell("That isn't a Nearvolt code.");
    }
    await _connect(device);
  }

  void _tell(String message) {
    if (mounted) ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(message)));
  }

  Future<void> _send() async {
    final (target, selection) = (_target, _selection);
    if (target == null || selection == null) return;
    _pause();
    final sent = await Navigator.of(context).push(MaterialPageRoute<bool>(
      builder: (_) => TransferPage(
        progress: engine.send(target, selection),
        peerName: target.name,
        link: target.link,
        incoming: false,
        onStop: engine.stopTransfer,
      ),
    ));
    if (!mounted) return;
    // A stopped transfer keeps its selection so sending again resumes it.
    if (sent ?? false) {
      setState(() {
        _selection = null;
        _target = null;
      });
    }
    _listen();
  }

  @override
  Widget build(BuildContext context) {
    final selection = _selection;
    return PopScope(
      canPop: selection == null,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) setState(() => _selection = null);
      },
      child: selection == null ? _start(context) : _ready(context, selection),
    );
  }

  List<Widget> get _notice => [
        if (needNotice case final notice?) ...[const SizedBox(height: Space.l), notice],
      ];

  Widget _start(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return PageScaffold(
      title: 'Send',
      onBack: () => Navigator.of(context).maybePop(),
      body: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            ..._notice,
            const SizedBox(height: Space.xl),
            Text('Nearby', style: text.titleMedium),
            const SizedBox(height: Space.m),
            DeviceList(devices: _devices, onTap: _choose),
            const SizedBox(height: Space.l),
            ConnectOptions(onScan: _scan, hasNfc: widget.profile.hasNfc),
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

  Widget _ready(BuildContext context, Selection selection) {
    final text = Theme.of(context).textTheme;
    final target = _target;
    return PageScaffold(
      title: 'Ready to send',
      onBack: () => setState(() => _selection = null),
      body: ListView(
        padding: const EdgeInsets.only(bottom: Space.xl),
        children: [
          ..._notice,
          const SizedBox(height: Space.l),
          SummaryCard(files: selection.files.length, bytes: selection.bytes),
          const SizedBox(height: Space.xxl),
          Text('Send to', style: text.titleMedium),
          const SizedBox(height: Space.s),
          DeviceList(devices: _withTarget(), selectedId: target?.id, onTap: (d) => setState(() => _target = d)),
          const SizedBox(height: Space.l),
          ConnectOptions(onScan: _scan, hasNfc: widget.profile.hasNfc),
          const SizedBox(height: Space.xxl),
          Text('Files', style: text.titleMedium),
          const SizedBox(height: Space.xs),
          for (final item in selection.items) FileRow(item: item),
        ],
      ),
      bottom: FilledButton(
        onPressed: target == null ? null : _send,
        child: Text(target == null ? 'Choose who to send to' : 'Send to ${target.name}'),
      ),
    );
  }

  /// A receiver chosen by QR code or tap stays visible even if discovery hasn't found it.
  List<NearbyDevice> _withTarget() {
    final target = _target;
    if (target == null || _devices.any((d) => d.id == target.id)) return _devices;
    return [target, ..._devices];
  }
}
