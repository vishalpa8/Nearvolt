import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/molecules.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'receive_page.dart';
import 'transfer_page.dart';

/// Start page: choose what to send, then who to send it to.
class SendPage extends StatefulWidget {
  const SendPage({super.key, required this.engine, required this.profile});

  final TransferEngine engine;
  final DeviceProfile profile;

  @override
  State<SendPage> createState() => _SendPageState();
}

class _SendPageState extends State<SendPage> {
  List<NearbyDevice> _devices = const [];
  Selection? _selection;
  NearbyDevice? _target;
  String? _update;
  StreamSubscription<List<NearbyDevice>>? _nearby;
  StreamSubscription<String>? _taps;

  TransferEngine get _engine => widget.engine;

  @override
  void initState() {
    super.initState();
    _listen();
    _engine.checkForUpdate().then((version) {
      if (mounted) setState(() => _update = version);
    });
  }

  /// Discovery and NFC run only while this page is visible.
  void _listen() {
    _nearby = _engine.nearby().listen((devices) => setState(() => _devices = devices));
    _taps = _engine.taps().listen((ticket) => _connect(_engine.deviceFor(ticket)));
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

  Future<void> _pick({bool folder = false}) async {
    final selection = folder ? await _engine.pickFolder() : await _engine.pickFiles();
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
    final ticket = await _engine.scanCode();
    if (ticket == null || !mounted) return;
    try {
      await _connect(_engine.deviceFor(ticket));
    } on Object {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(const SnackBar(content: Text("That isn't a Nearvolt code")));
      }
    }
  }

  Future<void> _send() async {
    final (target, selection) = (_target, _selection);
    if (target == null || selection == null) return;
    _pause();
    final sent = await Navigator.of(context).push(MaterialPageRoute<bool>(
      builder: (_) => TransferPage(
        progress: _engine.send(target, selection),
        peerName: target.name,
        link: target.link,
        incoming: false,
        onStop: _engine.stopTransfer,
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

  Future<void> _openReceive() async {
    _pause();
    await Navigator.of(context).push(MaterialPageRoute<void>(
      builder: (_) => ReceivePage(engine: _engine, profile: widget.profile),
    ));
    if (mounted) _listen();
  }

  @override
  Widget build(BuildContext context) => _selection == null ? _start(context) : _ready(context, _selection!);

  Widget _start(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return PageScaffold(
      title: 'Nearvolt',
      trailing: TextButton.icon(
        onPressed: _openReceive,
        icon: const Icon(Icons.download_rounded, size: 20),
        label: const Text('Receive'),
      ),
      body: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (_update case final version?) ...[
              const SizedBox(height: Space.l),
              UpdateBanner(version: version, install: _engine.installUpdate),
            ],
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
            DeviceList(devices: _devices, onTap: _choose),
            const SizedBox(height: Space.l),
            ConnectOptions(onScan: _scan, hasNfc: widget.profile.hasNfc),
          ],
        ),
      ),
      bottom: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          FilledButton.icon(
            onPressed: _pick,
            icon: const Icon(Icons.add_rounded),
            label: const Text('Choose files'),
          ),
          const SizedBox(height: Space.s),
          TextButton(onPressed: () => _pick(folder: true), child: const Text('Choose a folder')),
        ],
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
        padding: const EdgeInsets.only(top: Space.l, bottom: Space.xl),
        children: [
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
