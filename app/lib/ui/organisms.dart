import 'package:flutter/services.dart' show PlatformException;
import 'package:material_ui/material_ui.dart';

import '../engine/models.dart';
import '../theme.dart';
import 'atoms.dart';
import 'format.dart';
import 'molecules.dart';

/// Nearby devices, or a quiet searching state while none are found.
class DeviceList extends StatelessWidget {
  const DeviceList({super.key, required this.devices, required this.onTap, this.selectedId});

  final List<NearbyDevice> devices;
  final ValueChanged<NearbyDevice> onTap;
  final String? selectedId;

  @override
  Widget build(BuildContext context) {
    return AnimatedSwitcher(
      duration: const Duration(milliseconds: 250),
      layoutBuilder: (current, previous) =>
          Stack(alignment: AlignmentDirectional.topStart, children: [...previous, ?current]),
      child: devices.isEmpty
          ? const _Searching()
          : Wrap(
              key: const ValueKey('devices'),
              spacing: Space.s,
              runSpacing: Space.l,
              children: [
                for (final device in devices)
                  DeviceTile(device: device, selected: device.id == selectedId, onTap: () => onTap(device)),
              ],
            ),
    );
  }
}

class _Searching extends StatelessWidget {
  const _Searching();

  @override
  Widget build(BuildContext context) {
    return Padding(
      key: const ValueKey('searching'),
      padding: const EdgeInsets.symmetric(vertical: Space.l),
      child: Row(
        children: [
          const SizedBox.square(
            dimension: 18,
            child: CircularProgressIndicator(strokeWidth: 2, color: AppColors.accent),
          ),
          const SizedBox(width: Space.m),
          Expanded(
            child: Text(
              'Looking for devices with Receive open…',
              style: Theme.of(context).textTheme.bodyMedium?.copyWith(color: AppColors.textSecondary),
            ),
          ),
        ],
      ),
    );
  }
}

/// Speed, time left and file count for a running transfer.
class StatsCard extends StatelessWidget {
  const StatsCard({super.key, required this.progress});

  final TransferProgress? progress;

  @override
  Widget build(BuildContext context) {
    final p = progress;
    final speed = p == null || p.bytesPerSecond <= 0 ? '—' : formatSpeed(p.bytesPerSecond);
    final left = p?.timeLeft;
    final files = p == null ? '—' : '${formatCount(p.doneFiles)} / ${formatCount(p.totalFiles)}';
    return Panel(
      padding: const EdgeInsets.symmetric(vertical: Space.l),
      child: IntrinsicHeight(
        child: Row(
          children: [
            StatCell(label: 'Speed', value: speed),
            const VerticalDivider(),
            StatCell(label: 'Time left', value: left == null ? '—' : formatDuration(left)),
            const VerticalDivider(),
            StatCell(label: 'Files', value: files),
          ],
        ),
      ),
    );
  }
}

/// Asks whether to accept an incoming transfer. Resolves to true if accepted.
Future<bool> showReceivePrompt(BuildContext context, IncomingOffer offer) async {
  final accepted = await showModalBottomSheet<bool>(
    context: context,
    isDismissible: false,
    isScrollControlled: true,
    builder: (_) => _ReceivePrompt(offer: offer),
  );
  return accepted ?? false;
}

class _ReceivePrompt extends StatelessWidget {
  const _ReceivePrompt({required this.offer});

  final IncomingOffer offer;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final code = offer.code;
    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(Space.xl, 0, Space.xl, Space.xl),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            DeviceIcon(kind: offer.from.kind, size: 72),
            const SizedBox(height: Space.l),
            Text(
              '${offer.from.name} wants to send you',
              style: text.bodyLarge?.copyWith(color: AppColors.textSecondary),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: Space.xs),
            Text('${filesLabel(offer.files)} · ${formatBytes(offer.bytes)}', style: text.titleLarge),
            if (code != null) ...[
              const SizedBox(height: Space.l),
              CodeBadge(code: code, caption: 'Check that the sender sees the same code'),
            ],
            const SizedBox(height: Space.s),
            Text(
              'Saved to Downloads › Nearvolt. Every file is checked as it arrives.',
              style: text.bodySmall,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: Space.xl),
            Row(
              children: [
                Expanded(
                  child: OutlinedButton(
                    onPressed: () => Navigator.of(context).pop(false),
                    child: const Text('Decline'),
                  ),
                ),
                const SizedBox(width: Space.m),
                Expanded(
                  child: FilledButton(
                    onPressed: () => Navigator.of(context).pop(true),
                    child: const Text('Accept'),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// The other ways to reach a receiver: scan its code, or tap the phones together.
class ConnectOptions extends StatelessWidget {
  const ConnectOptions({super.key, required this.onScan, required this.hasNfc});

  final VoidCallback onScan;
  final bool hasNfc;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Row(
      children: [
        OutlinedButton.icon(
          onPressed: onScan,
          style: OutlinedButton.styleFrom(minimumSize: const Size(0, 44)),
          icon: const Icon(Icons.qr_code_scanner_rounded, size: 20),
          label: const Text('Scan code'),
        ),
        if (hasNfc) ...[
          const SizedBox(width: Space.m),
          const Icon(Icons.nfc_rounded, size: 20, color: AppColors.textSecondary),
          const SizedBox(width: Space.xs),
          Expanded(child: Text('or tap phones together', style: text.bodySmall)),
        ],
      ],
    );
  }
}

/// Offers a new version, then shows its download progress.
class UpdateBanner extends StatefulWidget {
  const UpdateBanner({super.key, required this.version, required this.install});

  final String version;
  final Stream<double> Function() install;

  @override
  State<UpdateBanner> createState() => _UpdateBannerState();
}

class _UpdateBannerState extends State<UpdateBanner> {
  double? _progress;
  String? _error;

  void _start() {
    setState(() {
      _progress = 0;
      _error = null;
    });
    widget.install().listen(
      (p) => setState(() => _progress = p),
      onError: (Object e) => setState(() {
        _progress = null;
        _error = e is PlatformException ? e.message : 'The update failed. Try again.';
      }),
    );
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final progress = _progress;
    return Panel(
      padding: const EdgeInsets.fromLTRB(Space.l, Space.s, Space.s, Space.s),
      child: Row(
        children: [
          const Icon(Icons.system_update_rounded, color: AppColors.accent),
          const SizedBox(width: Space.m),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('Version ${widget.version} is ready', style: text.labelLarge),
                if (progress != null) ...[
                  const SizedBox(height: Space.xs),
                  LinearProgressIndicator(value: progress, color: AppColors.accent),
                ] else if (_error != null)
                  Text(_error!, style: text.bodySmall?.copyWith(color: AppColors.danger)),
              ],
            ),
          ),
          if (progress == null) TextButton(onPressed: _start, child: const Text('Update')),
        ],
      ),
    );
  }
}

/// The 4-digit code both devices show when they connect without a QR code or tap.
class CodeBadge extends StatelessWidget {
  const CodeBadge({super.key, required this.code, required this.caption});

  final String code;
  final String caption;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Column(
      children: [
        Text(
          code.split('').join(' '),
          style: text.displaySmall?.copyWith(fontFeatures: tabularFigures, color: AppColors.accent),
        ),
        const SizedBox(height: Space.xs),
        Text(caption, style: text.bodySmall, textAlign: TextAlign.center),
      ],
    );
  }
}
