import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../debug.dart';
import '../engine/engine.dart';
import '../engine/messages.dart';
import '../engine/models.dart';
import '../theme.dart';
import 'atoms.dart';
import 'format.dart';
import 'molecules.dart';

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
        // Room above the phone icon: with none, it sat right on the sheet's curved edge.
        padding: const EdgeInsets.fromLTRB(Space.xl, Space.xl, Space.xl, Space.xl),
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

/// What a missing switch or permission looks like to the user: a picture, what is
/// wrong and why it matters, and the one button that fixes it.
(IconData, String, String) needDetails(Need need) => switch (need) {
  Need.permission => (
    Icons.devices_other_rounded,
    'Allow Nearby devices so Nearvolt can open its fast Wi-Fi link between phones.',
    'Allow',
  ),
  Need.permissionInSettings => (
    Icons.devices_other_rounded,
    'Nearby devices is off for Nearvolt. Turn it on under Permissions.',
    'Settings',
  ),
  Need.nfc => (Icons.nfc_rounded, 'NFC is off. Turn it on so phones can connect by tapping.', 'Turn on'),
  Need.wifi => (Icons.wifi_off_rounded, 'Wi-Fi is off. Nearvolt needs it to send and receive.', 'Turn on'),
  Need.location => (Icons.location_off_rounded, 'This Android version needs Location on to connect phones.', 'Turn on'),
};

/// Shown instead of a page while something it needs is off. The page, and what it
/// would start (the network, NFC, the camera), stays closed until every
/// need is met; the page opens by itself as soon as the last one is turned on.
class NeedsGate extends StatelessWidget {
  const NeedsGate({super.key, required this.needs, required this.onFix});

  final List<Need> needs;
  final void Function(Need need) onFix;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Center(
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('Turn these on to continue', style: text.titleLarge, textAlign: TextAlign.center),
            const SizedBox(height: Space.xs),
            Text(
              'Already on? This checks again by itself.',
              style: text.bodySmall?.copyWith(color: AppColors.textSecondary),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: Space.l),
            for (final need in needs) ...[
              Panel(
                padding: const EdgeInsets.fromLTRB(Space.l, Space.s, Space.s, Space.s),
                child: Row(
                  children: [
                    Icon(needDetails(need).$1, color: AppColors.warning),
                    const SizedBox(width: Space.m),
                    Expanded(child: Text(needDetails(need).$2, style: text.bodyMedium)),
                    TextButton(onPressed: () => onFix(need), child: Text(needDetails(need).$3)),
                  ],
                ),
              ),
              const SizedBox(height: Space.s),
            ],
          ],
        ),
      ),
    );
  }
}

/// Before a screen that opens the camera or another radio: asks for whatever is
/// still off and keeps asking until it is on. Returns false if the user backs out.
Future<bool> ensureReady(BuildContext context, TransferEngine engine, Purpose purpose) async {
  while (true) {
    final missing = await engine.needs(purpose);
    if (missing.isEmpty) return true;
    if (!context.mounted) return false;
    final need = missing.first;
    final (_, message, action) = needDetails(need);
    final turnOn = await showDialog<bool>(
      context: context,
      barrierDismissible: false,
      builder: (context) => AlertDialog(
        content: Text(message),
        actions: [
          TextButton(onPressed: () => Navigator.of(context).pop(false), child: const Text('Not now')),
          TextButton(onPressed: () => Navigator.of(context).pop(true), child: Text(action)),
        ],
      ),
    );
    if (turnOn != true) return false;
    await engine.fix(need);
  }
}

enum PickKind { files, folder }

/// A short message at the bottom of the screen, also written to the log so what the
/// user was told can be traced.
void showNote(BuildContext context, String message) {
  DebugLog.log('message', message);
  ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(message)));
}

/// Picks files or a folder. A big folder takes a moment to read, so after a short
/// wait a notice says so, instead of the screen looking frozen.
Future<Selection?> pickSelection(BuildContext context, TransferEngine engine, PickKind kind) async {
  final picking = kind == PickKind.folder ? engine.pickFolder() : engine.pickFiles();
  var notice = false;
  final navigator = Navigator.of(context, rootNavigator: true);
  final timer = Timer(const Duration(milliseconds: 600), () {
    if (!context.mounted) return;
    notice = true;
    showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (_) => const PopScope(
        canPop: false,
        child: AlertDialog(
          content: Row(
            children: [
              SizedBox(width: 24, height: 24, child: CircularProgressIndicator(strokeWidth: 3)),
              SizedBox(width: Space.l),
              Expanded(child: Text('Reading your files…')),
            ],
          ),
        ),
      ),
    );
  });
  try {
    return await picking;
  } finally {
    timer.cancel();
    if (notice) navigator.pop();
  }
}

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

/// Every batch on a connection, newest first. Each lists its files by name, with a bar
/// for the one moving, and lets the user cancel the whole batch or skip a single file.
class BatchList extends StatelessWidget {
  const BatchList({super.key, required this.batches, required this.sending, this.onCancelBatch, this.onSkipFile});

  final List<Batch> batches;

  /// The sender says "Sent"; the receiver says "Received".
  final bool sending;

  /// Null once the connection is over, so nothing offers to cancel.
  final void Function(Batch batch)? onCancelBatch;
  final void Function(Batch batch, int file)? onSkipFile;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final batch in batches.reversed)
          _BatchCard(
            key: ValueKey('batch-${batch.id}'),
            batch: batch,
            sending: sending,
            onCancel: onCancelBatch == null ? null : () => onCancelBatch!(batch),
            onSkip: onSkipFile == null ? null : (file) => onSkipFile!(batch, file),
          ),
      ],
    );
  }
}

/// Rows shown for one batch before the list is cut down to what's happening now.
const _rowsShown = 40;

class _BatchCard extends StatelessWidget {
  const _BatchCard({super.key, required this.batch, required this.sending, this.onCancel, this.onSkip});

  final Batch batch;
  final bool sending;
  final VoidCallback? onCancel;
  final void Function(int file)? onSkip;

  bool get _live => batch.state == BatchState.queued || batch.state == BatchState.moving;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final rows = batch.rows;
    // Finished batches of many files collapse to their summary line.
    final showRows = rows.isNotEmpty && (_live || rows.length <= _rowsShown);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Space.s),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
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
                        BatchState.cancelled => sending ? 'Cancelled · not sent' : 'Cancelled · not received',
                      },
                      style: text.bodySmall?.copyWith(
                        color: batch.state == BatchState.failed
                            ? readableOn(AppColors.danger, AppColors.background)
                            : null,
                      ),
                    ),
                  ],
                ),
              ),
              if (_live && onCancel != null) TextButton(onPressed: onCancel, child: const Text('Cancel batch')),
            ],
          ),
          if (showRows) ...[const SizedBox(height: Space.xs), ..._fileRows(context, rows)],
        ],
      ),
    );
  }

  /// All rows of a small batch; of a big one, the files around the one moving, with
  /// how many are done above and waiting below.
  List<Widget> _fileRows(BuildContext context, List<BatchFile> rows) {
    var from = 0;
    var to = rows.length;
    if (rows.length > _rowsShown) {
      final firstOpen = rows.indexWhere((r) => r.state != FileState.done && r.state != FileState.skipped);
      from = firstOpen < 0 ? 0 : firstOpen;
      to = (from + _rowsShown).clamp(0, rows.length);
    }
    final text = Theme.of(context).textTheme;
    Widget note(String label) => Padding(
      padding: const EdgeInsets.only(left: 44 + Space.m, top: Space.xs, bottom: Space.xs),
      child: Text(label, style: text.bodySmall?.copyWith(color: AppColors.textSecondary)),
    );
    final listed = rows.length;
    return [
      if (from > 0) note(from == 1 ? '1 file already done' : '$from files already done'),
      for (var i = from; i < to; i++)
        FileTile(
          key: ValueKey('file-${batch.id}-$i'),
          row: rows[i],
          sending: sending,
          batchLive: _live,
          onSkip: onSkip == null ? null : () => onSkip!(i),
        ),
      if (to < listed) note(listed - to == 1 ? '1 more waiting' : '${listed - to} more waiting'),
      if (batch.files > listed) note('and ${batch.files - listed} more that aren\'t listed here'),
    ];
  }
}

/// One file of a batch: its name, how far it is, and a button to skip it.
class FileTile extends StatelessWidget {
  const FileTile({super.key, required this.row, required this.sending, required this.batchLive, this.onSkip});

  final BatchFile row;
  final bool sending;
  final bool batchLive;
  final VoidCallback? onSkip;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final skippable = batchLive && onSkip != null && (row.state == FileState.queued || row.state == FileState.moving);
    final (icon, color) = switch (row.state) {
      FileState.done => (Icons.check_circle_rounded, AppColors.success),
      FileState.moving => (Icons.sync_rounded, AppColors.accent),
      FileState.skipped => (Icons.block_rounded, AppColors.warning),
      FileState.queued => (batchLive ? Icons.schedule_rounded : Icons.block_rounded, AppColors.textSecondary),
    };
    final detail = switch (row.state) {
      FileState.done => formatBytes(row.size),
      FileState.moving => '${formatBytes(row.bytes)} of ${formatBytes(row.size)}',
      FileState.skipped => 'Skipped · ${sending ? 'not sent' : 'not received'}',
      FileState.queued =>
        batchLive
            ? '${formatBytes(row.size)} · waiting'
            : '${formatBytes(row.size)} · not ${sending ? 'sent' : 'received'}',
    };
    return Semantics(
      label: '${row.name}, $detail',
      child: Padding(
        padding: const EdgeInsets.only(left: 44 + Space.m - 22, top: 2, bottom: 2),
        child: Row(
          children: [
            Icon(icon, size: 20, color: color),
            const SizedBox(width: Space.m),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(row.name.split('/').last, maxLines: 1, overflow: TextOverflow.ellipsis, style: text.bodyMedium),
                  const SizedBox(height: 2),
                  if (row.state == FileState.moving)
                    ClipRRect(
                      borderRadius: BorderRadius.circular(2),
                      child: LinearProgressIndicator(
                        value: row.fraction,
                        minHeight: 4,
                        color: AppColors.accent,
                        backgroundColor: AppColors.accentSoft,
                      ),
                    ),
                  Text(detail, style: text.bodySmall?.copyWith(color: AppColors.textSecondary)),
                ],
              ),
            ),
            if (skippable)
              IconButton(
                onPressed: onSkip,
                tooltip: 'Skip ${row.name.split('/').last}',
                icon: const Icon(Icons.close_rounded, size: 20, color: AppColors.textSecondary),
              ),
          ],
        ),
      ),
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
      BatchState.cancelled => (Icons.block_rounded, AppColors.warning, AppColors.warning.withValues(alpha: 0.12)),
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
    this.onDone,
  });

  /// The installed version.
  final String current;
  final Future<String?> Function() check;
  final Stream<double> Function() install;
  final VoidCallback onClose;

  /// Called once the check, and any install it started, has ended one way or another.
  final VoidCallback? onDone;

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
      await _checkAndInstall();
    } finally {
      widget.onDone?.call();
    }
  }

  Future<void> _checkAndInstall() async {
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
