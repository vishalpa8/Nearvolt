import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/models.dart';
import '../theme.dart';
import '../ui/atoms.dart';
import '../ui/format.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';

class TransferPage extends StatefulWidget {
  const TransferPage({super.key, required this.progress, required this.peer, required this.incoming});

  final Stream<TransferProgress> progress;
  final NearbyDevice peer;
  final bool incoming;

  @override
  State<TransferPage> createState() => _TransferPageState();
}

class _TransferPageState extends State<TransferPage> {
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

  bool get _finished => switch (_progress?.phase) {
        TransferPhase.completed || TransferPhase.failed => true,
        _ => false,
      };

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
    final color = switch (phase) {
      TransferPhase.completed => AppColors.success,
      TransferPhase.reconnecting => AppColors.warning,
      TransferPhase.failed => AppColors.danger,
      _ => AppColors.accent,
    };
    return PopScope(
      canPop: _finished,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _cancel();
      },
      child: PageScaffold(
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SizedBox(height: Space.l),
            Text(
              widget.incoming ? 'Receiving from' : 'Sending to',
              style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
            ),
            const SizedBox(height: 2),
            Text(widget.peer.name, style: text.titleLarge),
            const SizedBox(height: Space.m),
            Align(alignment: Alignment.centerLeft, child: Pill(label: _statusLabel(phase), color: color)),
            const Spacer(),
            Center(
              child: ProgressRing(
                fraction: p?.fraction ?? 0,
                color: color,
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
            if (done) _Done(files: p!.totalFiles, bytes: p.totalBytes) else StatsCard(progress: p),
          ],
        ),
        bottom: _finished
            ? FilledButton(
                onPressed: () => Navigator.of(context).popUntil((route) => route.isFirst),
                child: const Text('Done'),
              )
            : TextButton(onPressed: _cancel, child: const Text('Cancel')),
      ),
    );
  }

  String _statusLabel(TransferPhase phase) => switch (phase) {
        TransferPhase.connecting => 'Connecting…',
        TransferPhase.transferring => 'Connected · ${widget.peer.link} · Encrypted',
        TransferPhase.reconnecting => 'Link dropped · Reconnecting…',
        TransferPhase.completed => 'Complete',
        TransferPhase.failed => _progress?.error ?? 'Stopped',
      };
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
