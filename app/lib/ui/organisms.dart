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

/// The most important thing stopping Nearvolt from reaching other phones, with
/// the one button that fixes it.
class NeedNotice extends StatelessWidget {
  const NeedNotice({super.key, required this.need, required this.onFix});

  final Need need;
  final VoidCallback onFix;

  @override
  Widget build(BuildContext context) {
    final (icon, message, action) = switch (need) {
      Need.permission => (
          Icons.devices_other_rounded,
          'Allow Nearby devices so Nearvolt can find phones and be found.',
          'Allow',
        ),
      Need.permissionInSettings => (
          Icons.devices_other_rounded,
          'Nearby devices is off for Nearvolt. Turn it on under Permissions.',
          'Settings',
        ),
      Need.wifi => (Icons.wifi_off_rounded, 'Wi-Fi is off. Transfers need it, except over a USB cable.', 'Turn on'),
      Need.bluetooth => (
          Icons.bluetooth_disabled_rounded,
          "Bluetooth is off, so phones on another network won't show up.",
          'Turn on',
        ),
      Need.location => (Icons.location_off_rounded, 'This Android version needs Location on to find phones.', 'Turn on'),
    };
    return Panel(
      padding: const EdgeInsets.fromLTRB(Space.l, Space.s, Space.s, Space.s),
      child: Row(
        children: [
          Icon(icon, color: AppColors.warning),
          const SizedBox(width: Space.m),
          Expanded(child: Text(message, style: Theme.of(context).textTheme.bodyMedium)),
          TextButton(onPressed: onFix, child: Text(action)),
        ],
      ),
    );
  }
}

enum PickKind { files, folder }

/// Asks whether to pick files or a whole folder.
Future<PickKind?> showPickMenu(BuildContext context) => showModalBottomSheet<PickKind>(
      context: context,
      builder: (context) => SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(Space.s, 0, Space.s, Space.l),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                leading: const Icon(Icons.insert_drive_file_rounded, color: AppColors.accent),
                title: const Text('Files'),
                subtitle: const Text('Photos, videos, documents and more'),
                onTap: () => Navigator.of(context).pop(PickKind.files),
              ),
              ListTile(
                leading: const Icon(Icons.folder_rounded, color: AppColors.accent),
                title: const Text('Folder'),
                subtitle: const Text('Everything inside, with its folders'),
                onTap: () => Navigator.of(context).pop(PickKind.folder),
              ),
            ],
          ),
        ),
      ),
    );

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

enum _UpdateStep { checking, latest, downloading, installing, failed }

/// Checks for a new version when shown, then downloads and installs it
/// straight away.
class UpdateCard extends StatefulWidget {
  const UpdateCard({
    super.key,
    required this.current,
    required this.check,
    required this.install,
    required this.onClose,
  });

  /// The installed version.
  final String current;
  final Future<String?> Function() check;
  final Stream<double> Function() install;
  final VoidCallback onClose;

  @override
  State<UpdateCard> createState() => _UpdateCardState();
}

class _UpdateCardState extends State<UpdateCard> {
  var _step = _UpdateStep.checking;
  String? _version;
  double _progress = 0;
  String? _error;

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    try {
      final version = await widget.check();
      if (!mounted) return;
      if (version == null) return setState(() => _step = _UpdateStep.latest);
      setState(() {
        _version = version;
        _step = _UpdateStep.downloading;
      });
      await for (final progress in widget.install()) {
        if (mounted) setState(() => _progress = progress);
      }
      if (mounted) setState(() => _step = _UpdateStep.installing);
    } on Object catch (e) {
      if (!mounted) return;
      setState(() {
        _step = _UpdateStep.failed;
        _error = e is PlatformException ? e.message : null;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final message = switch (_step) {
      _UpdateStep.checking => 'Checking for updates…',
      _UpdateStep.latest => 'Nearvolt ${widget.current} is the latest version.',
      _UpdateStep.downloading => 'Downloading version $_version…',
      _UpdateStep.installing => 'Installing version $_version. Nearvolt closes for a moment; open it again afterwards.',
      _UpdateStep.failed => _error ?? "The update didn't finish. Try again.",
    };
    final busy = _step == _UpdateStep.checking || _step == _UpdateStep.installing;
    return Panel(
      padding: const EdgeInsets.fromLTRB(Space.l, Space.m, Space.s, Space.m),
      child: Row(
        children: [
          Icon(
            _step == _UpdateStep.failed ? Icons.error_outline_rounded : Icons.system_update_rounded,
            color: _step == _UpdateStep.failed ? AppColors.danger : AppColors.accent,
          ),
          const SizedBox(width: Space.m),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(message, style: text.bodyMedium),
                if (_step == _UpdateStep.downloading) ...[
                  const SizedBox(height: Space.s),
                  LinearProgressIndicator(
                    value: _progress,
                    color: AppColors.accent,
                    backgroundColor: AppColors.surfaceMuted,
                    borderRadius: BorderRadius.circular(Radii.button),
                  ),
                ],
              ],
            ),
          ),
          const SizedBox(width: Space.s),
          if (busy)
            const Padding(
              padding: EdgeInsets.all(Space.m),
              child: SizedBox.square(
                dimension: 18,
                child: CircularProgressIndicator(strokeWidth: 2, color: AppColors.accent),
              ),
            )
          else if (_step != _UpdateStep.downloading)
            IconButton(
              onPressed: widget.onClose,
              tooltip: 'Close',
              icon: const Icon(Icons.close_rounded, color: AppColors.textSecondary),
            ),
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
