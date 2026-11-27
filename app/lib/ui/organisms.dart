import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/messages.dart';
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

/// Speed, time left and file count for the batch on the move.
class StatsCard extends StatelessWidget {
  const StatsCard({super.key, required this.progress});

  final Progress? progress;

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

/// Asks whether to let a phone connect. Resolves to true if accepted, false if
/// declined, and null if the phone left first: the sheet then closes by itself.
Future<bool?> showReceivePrompt(BuildContext context, Request request, {required Stream<void> withdrawn}) {
  return showModalBottomSheet<bool>(
    context: context,
    isDismissible: false,
    enableDrag: false,
    isScrollControlled: true,
    builder: (_) => _ReceivePrompt(request: request, withdrawn: withdrawn),
  );
}

class _ReceivePrompt extends StatefulWidget {
  const _ReceivePrompt({required this.request, required this.withdrawn});

  final Request request;
  final Stream<void> withdrawn;

  @override
  State<_ReceivePrompt> createState() => _ReceivePromptState();
}

class _ReceivePromptState extends State<_ReceivePrompt> {
  late final StreamSubscription<void> _sub;
  NavigatorState? _navigator;
  Route<dynamic>? _route;

  @override
  void initState() {
    super.initState();
    // The phone left: take this sheet, and only this sheet, off the screen.
    _sub = widget.withdrawn.listen((_) {
      final (navigator, route) = (_navigator, _route);
      if (mounted && navigator != null && route != null) navigator.removeRoute(route);
    });
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _navigator = Navigator.of(context);
    _route = ModalRoute.of(context);
  }

  @override
  void dispose() {
    _sub.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final request = widget.request;
    final text = Theme.of(context).textTheme;
    final code = request.code;
    // Only the buttons answer; the back gesture must not decline by accident.
    return PopScope(canPop: false, child: _sheet(context, request, text, code));
  }

  Widget _sheet(BuildContext context, Request request, TextTheme text, String? code) {
    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(Space.xl, 0, Space.xl, Space.xl),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const DeviceIcon(kind: DeviceKind.phone, size: 72),
            const SizedBox(height: Space.l),
            Text(request.from, style: text.titleLarge, textAlign: TextAlign.center),
            const SizedBox(height: Space.xs),
            Text(
              'wants to connect and send you files',
              style: text.bodyLarge?.copyWith(color: AppColors.textSecondary),
              textAlign: TextAlign.center,
            ),
            if (code != null) ...[
              const SizedBox(height: Space.l),
              CodeBadge(code: code, caption: 'Check that the other phone shows the same code'),
            ],
            const SizedBox(height: Space.l),
            Text(
              'Files are saved to Downloads › Nearvolt and checked as they arrive. '
              'You stay connected until either phone disconnects.',
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
                  child: FilledButton(onPressed: () => Navigator.of(context).pop(true), child: const Text('Accept')),
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
      Need.nfc => (Icons.nfc_rounded, 'NFC is off. Turn it on to tap phones together.', 'Turn on'),
      Need.wifi => (Icons.wifi_off_rounded, 'Wi-Fi is off. Transfers need it, except over a USB cable.', 'Turn on'),
      Need.bluetooth => (
        Icons.bluetooth_disabled_rounded,
        "Bluetooth is off, so phones on another network won't show up.",
        'Turn on',
      ),
      Need.location => (
        Icons.location_off_rounded,
        'This Android version needs Location on to find phones.',
        'Turn on',
      ),
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

/// Every batch on a connection, oldest first, with what happened to it.
class BatchList extends StatelessWidget {
  const BatchList({super.key, required this.batches, required this.sending});

  final List<Batch> batches;

  /// The sender says "Sent"; the receiver says "Received".
  final bool sending;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final batch in batches.reversed)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: Space.s),
            child: Row(
              children: [
                _BatchIcon(state: batch.state),
                const SizedBox(width: Space.m),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('${filesLabel(batch.files)} · ${formatBytes(batch.bytes)}', style: text.bodyLarge),
                      Text(
                        switch (batch.state) {
                          BatchState.queued => 'Waiting its turn',
                          BatchState.moving => sending ? 'Sending' : 'Receiving',
                          BatchState.done => sending ? 'Sent and verified' : 'Received and verified',
                          BatchState.failed => batch.error ?? "Couldn't be sent",
                        },
                        style: text.bodySmall?.copyWith(
                          color: batch.state == BatchState.failed ? AppColors.danger : null,
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

class _BatchIcon extends StatelessWidget {
  const _BatchIcon({required this.state});

  final BatchState state;

  @override
  Widget build(BuildContext context) {
    final (icon, color, background) = switch (state) {
      BatchState.queued => (Icons.schedule_rounded, AppColors.textSecondary, AppColors.surfaceMuted),
      BatchState.moving => (Icons.swap_vert_rounded, AppColors.accent, AppColors.accentSoft),
      BatchState.done => (Icons.check_rounded, AppColors.success, AppColors.success.withValues(alpha: 0.1)),
      BatchState.failed => (Icons.close_rounded, AppColors.danger, AppColors.danger.withValues(alpha: 0.1)),
    };
    return Container(
      width: 44,
      height: 44,
      decoration: BoxDecoration(color: background, borderRadius: BorderRadius.circular(Radii.button)),
      child: Icon(icon, color: color, size: 22),
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
      // Reaching 1 means the APK is ready and Android is installing it; the stream
      // ends with an error if Android refuses.
      await for (final progress in widget.install()) {
        if (!mounted) continue;
        setState(() {
          _progress = progress;
          if (progress >= 1) _step = _UpdateStep.installing;
        });
      }
    } on Object catch (e) {
      if (!mounted) return;
      setState(() {
        _step = _UpdateStep.failed;
        _error = friendlyPlatform(e);
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
