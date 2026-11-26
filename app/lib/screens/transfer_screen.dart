import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/models.dart';
import '../theme/tokens.dart';
import '../util/format.dart';
import '../widgets/progress_ring.dart';
import '../widgets/status_pill.dart';

class TransferScreen extends StatefulWidget {
  const TransferScreen({super.key, required this.progress, required this.peer, required this.incoming});

  final Stream<TransferProgress> progress;
  final NearbyDevice peer;
  final bool incoming;

  @override
  State<TransferScreen> createState() => _TransferScreenState();
}

class _TransferScreenState extends State<TransferScreen> {
  TransferProgress? _progress;
  late final StreamSubscription<TransferProgress> _sub;

  @override
  void initState() {
    super.initState();
    _sub = widget.progress.listen((p) => setState(() => _progress = p));
  }

  @override
  void dispose() {
    _sub.cancel();
    super.dispose();
  }

  Future<void> _cancel() async {
    final stop = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Stop this transfer?'),
        content: const Text('Files already received stay safe, and you can resume later.'),
        actions: [
          TextButton(onPressed: () => Navigator.of(context).pop(false), child: const Text('Keep going')),
          TextButton(onPressed: () => Navigator.of(context).pop(true), child: const Text('Stop')),
        ],
      ),
    );
    if (stop == true && mounted) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final p = _progress;
    final phase = p?.phase ?? TransferPhase.connecting;
    final done = phase == TransferPhase.completed;
    final ringColor = switch (phase) {
      TransferPhase.completed => AppColors.success,
      TransferPhase.reconnecting => AppColors.warning,
      TransferPhase.failed => AppColors.danger,
      _ => AppColors.accent,
    };
    return PopScope(
      canPop: done,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _cancel();
      },
      child: Scaffold(
        body: SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(Space.xl, Space.l, Space.xl, Space.xl),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(
                  widget.incoming ? 'Receiving from' : 'Sending to',
                  style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
                ),
                const SizedBox(height: 2),
                Text(widget.peer.name, style: text.titleLarge),
                const SizedBox(height: Space.m),
                Align(alignment: Alignment.centerLeft, child: _status(phase)),
                const Spacer(),
                Center(
                  child: ProgressRing(
                    fraction: p?.fraction ?? 0,
                    color: ringColor,
                    child: done
                        ? const Icon(Icons.check_rounded, size: 72, color: AppColors.success)
                        : Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              Text(
                                '${((p?.fraction ?? 0) * 100).floor()}%',
                                style: text.displayLarge?.copyWith(fontFeatures: tabularFigures),
                              ),
                              const SizedBox(height: Space.xs),
                              Text(
                                p == null ? '' : '${formatBytes(p.doneBytes)} of ${formatBytes(p.totalBytes)}',
                                style: text.bodySmall?.copyWith(fontFeatures: tabularFigures),
                              ),
                            ],
                          ),
                  ),
                ),
                const Spacer(),
                if (done) _Done(files: p!.totalFiles, bytes: p.totalBytes) else _Stats(progress: p),
                const SizedBox(height: Space.xl),
                if (done)
                  FilledButton(
                    onPressed: () => Navigator.of(context).popUntil((route) => route.isFirst),
                    child: const Text('Done'),
                  )
                else
                  TextButton(onPressed: _cancel, child: const Text('Cancel')),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _status(TransferPhase phase) {
    return switch (phase) {
      TransferPhase.connecting => const StatusPill(label: 'Connecting…', color: AppColors.accent),
      TransferPhase.transferring =>
        StatusPill(label: 'Connected · ${widget.peer.link} · Encrypted', color: AppColors.success),
      TransferPhase.reconnecting => const StatusPill(label: 'Link dropped · Reconnecting…', color: AppColors.warning),
      TransferPhase.completed => const StatusPill(label: 'Complete', color: AppColors.success),
      TransferPhase.failed => StatusPill(label: _progress?.error ?? 'Stopped', color: AppColors.danger),
    };
  }
}

class _Stats extends StatelessWidget {
  const _Stats({required this.progress});

  final TransferProgress? progress;

  @override
  Widget build(BuildContext context) {
    final p = progress;
    final speed = p == null || p.bytesPerSecond <= 0 ? '—' : formatSpeed(p.bytesPerSecond);
    final left = p?.timeLeft;
    final files = p == null ? '—' : '${formatCount(p.doneFiles)} / ${formatCount(p.totalFiles)}';
    return Container(
      padding: const EdgeInsets.symmetric(vertical: Space.l),
      decoration: BoxDecoration(
        color: AppColors.surface,
        border: Border.all(color: AppColors.border),
        borderRadius: BorderRadius.circular(Radii.card),
      ),
      child: IntrinsicHeight(
        child: Row(
          children: [
            _Stat(label: 'Speed', value: speed),
            const VerticalDivider(),
            _Stat(label: 'Time left', value: left == null ? '—' : formatDuration(left)),
            const VerticalDivider(),
            _Stat(label: 'Files', value: files),
          ],
        ),
      ),
    );
  }
}

class _Stat extends StatelessWidget {
  const _Stat({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Expanded(
      child: Column(
        children: [
          Text(label, style: text.bodySmall),
          const SizedBox(height: Space.xs),
          Text(
            value,
            style: text.titleMedium?.copyWith(fontFeatures: tabularFigures),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
        ],
      ),
    );
  }
}

class _Done extends StatelessWidget {
  const _Done({required this.files, required this.bytes});

  final int files;
  final int bytes;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final subject = files == 1 ? 'The file' : 'All ${formatCount(files)} files';
    return Column(
      children: [
        Text('$subject verified', style: text.titleLarge, textAlign: TextAlign.center),
        const SizedBox(height: Space.xs),
        Text(
          '${formatBytes(bytes)}, every byte identical to the original.',
          style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
          textAlign: TextAlign.center,
        ),
      ],
    );
  }
}
