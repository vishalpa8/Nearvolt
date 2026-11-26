import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme/tokens.dart';
import '../util/format.dart';
import '../widgets/device_avatar.dart';
import '../widgets/file_tile.dart';
import 'transfer_screen.dart';

class PickScreen extends StatefulWidget {
  const PickScreen({super.key, required this.engine, required this.items, required this.devices, this.initial});

  final TransferEngine engine;
  final List<PickedItem> items;
  final List<NearbyDevice> devices;
  final NearbyDevice? initial;

  @override
  State<PickScreen> createState() => _PickScreenState();
}

class _PickScreenState extends State<PickScreen> {
  late final List<PickedItem> _items = [...widget.items];
  late NearbyDevice? _target = widget.initial;

  int get _bytes => _items.fold(0, (sum, item) => sum + item.bytes);
  int get _files => _items.fold(0, (sum, item) => sum + item.files);

  Future<void> _addMore() async {
    final more = await widget.engine.pickFiles();
    if (more.isNotEmpty) setState(() => _items.addAll(more));
  }

  void _send() {
    final target = _target;
    if (target == null || _items.isEmpty) return;
    Navigator.of(context).pushReplacement(MaterialPageRoute<void>(
      builder: (_) => TransferScreen(progress: widget.engine.send(target, _items), peer: target, incoming: false),
    ));
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final target = _target;
    return Scaffold(
      body: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(Space.s, Space.s, Space.xl, 0),
              child: Row(
                children: [
                  IconButton(
                    onPressed: () => Navigator.of(context).maybePop(),
                    tooltip: 'Back',
                    icon: const Icon(Icons.arrow_back_rounded),
                  ),
                  const SizedBox(width: Space.xs),
                  Text('Ready to send', style: text.titleMedium),
                ],
              ),
            ),
            Expanded(
              child: ListView(
                padding: const EdgeInsets.fromLTRB(Space.xl, Space.l, Space.xl, Space.xl),
                children: [
                  _Summary(files: _files, bytes: _bytes),
                  const SizedBox(height: Space.xxl),
                  Text('Send to', style: text.titleMedium),
                  const SizedBox(height: Space.s),
                  if (widget.devices.isEmpty)
                    Text('Looking for nearby devices…', style: text.bodyMedium?.copyWith(color: AppColors.textSecondary))
                  else
                    SingleChildScrollView(
                      scrollDirection: Axis.horizontal,
                      clipBehavior: Clip.none,
                      child: Row(
                        children: [
                          for (final device in widget.devices)
                            DeviceAvatar(
                              device: device,
                              selected: device.id == target?.id,
                              onTap: () => setState(() => _target = device),
                            ),
                        ],
                      ),
                    ),
                  const SizedBox(height: Space.xxl),
                  Row(
                    children: [
                      Text('Files', style: text.titleMedium),
                      const Spacer(),
                      TextButton.icon(
                        onPressed: _addMore,
                        icon: const Icon(Icons.add_rounded, size: 20),
                        label: const Text('Add more'),
                      ),
                    ],
                  ),
                  for (final item in _items)
                    FileTile(item: item, onRemove: () => setState(() => _items.remove(item))),
                ],
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(Space.xl, Space.s, Space.xl, Space.xl),
              child: FilledButton(
                onPressed: target == null || _items.isEmpty ? null : _send,
                child: Text(target == null ? 'Choose a device' : 'Send to ${target.name}'),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Summary extends StatelessWidget {
  const _Summary({required this.files, required this.bytes});

  final int files;
  final int bytes;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Container(
      padding: const EdgeInsets.all(Space.xl),
      decoration: BoxDecoration(
        color: AppColors.surface,
        border: Border.all(color: AppColors.border),
        borderRadius: BorderRadius.circular(Radii.card),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(formatBytes(bytes), style: text.displaySmall?.copyWith(fontFeatures: tabularFigures)),
          const SizedBox(height: Space.xs),
          Text(filesLabel(files), style: text.bodyLarge?.copyWith(color: AppColors.textSecondary)),
        ],
      ),
    );
  }
}
