import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/models.dart';
import '../theme.dart';
import '../ui/atoms.dart';
import '../ui/format.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';

class TransferPage extends StatefulWidget {
  const TransferPage({
    super.key,
    required this.progress,
    required this.peerName,
    required this.incoming,
    required this.onStop,
    this.link,
  });

  final Stream<TransferProgress> progress;
  final String peerName;
  final String? link;
  final bool incoming;
  final VoidCallback onStop;

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

  bool get _finished => _progress?.finished ?? false;

  /// Pops with whether everything arrived.
  void _close() => Navigator.of(context).pop(_progress?.phase == TransferPhase.completed);

  Future<void> _cancel() async {
    final stop = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Stop this transfer?'),
        content: const Text('Files already received stay safe, and the transfer can resume later.'),
        actions: [
          TextButton(onPressed: () => Navigator.of(context).pop(false), child: const Text('Keep going')),
          TextButton(onPressed: () => Navigator.of(context).pop(true), child: const Text('Stop')),
        ],
      ),
    );
    if (stop != true || !mounted) return;
    widget.onStop();
    Navigator.of(context).pop(false);
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final p = _progress;
    final phase = p?.phase ?? TransferPhase.connecting;
    final color = switch (phase) {
      TransferPhase.completed => AppColors.success,
      TransferPhase.reconnecting => AppColors.warning,
      TransferPhase.failed => AppColors.danger,
      _ => AppColors.accent,
    };
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _finished ? _close() : _cancel();
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
            Text(widget.peerName, style: text.titleLarge),
            const SizedBox(height: Space.m),
            Align(alignment: Alignment.centerLeft, child: Pill(label: _status(phase), color: color)),
            const Spacer(),
            Center(
              child: ProgressRing(
                fraction: p?.fraction ?? 0,
                color: color,
                child: switch (phase) {
                  TransferPhase.completed => const Icon(Icons.check_rounded, size: 72, color: AppColors.success),
                  TransferPhase.failed => const Icon(Icons.close_rounded, size: 72, color: AppColors.danger),
                  _ => Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Text(
                          '${((p?.fraction ?? 0) * 100).floor()}%',
                          style: text.displayLarge?.copyWith(fontFeatures: tabularFigures),
                        ),
                        const SizedBox(height: Space.xs),
                        Text(
                          p == null || p.totalBytes == 0 ? '' : '${formatBytes(p.doneBytes)} of ${formatBytes(p.totalBytes)}',
                          style: text.bodySmall?.copyWith(fontFeatures: tabularFigures),
                        ),
                      ],
                    ),
                },
              ),
            ),
            const Spacer(),
            switch (phase) {
              TransferPhase.completed => _Outcome(
                  title: '${p!.totalFiles == 1 ? 'The file' : 'All ${formatCount(p.totalFiles)} files'} verified',
                  detail: '${formatBytes(p.totalBytes)}, every byte identical to the original.',
                ),
              TransferPhase.failed => _Outcome(
                  title: 'Transfer stopped',
                  detail: '${p?.error ?? 'The connection was lost.'} Files already received are kept.',
                ),
              TransferPhase.waiting when p?.code != null => CodeBadge(
                  code: p!.code!,
                  caption: 'Check that ${widget.peerName} shows the same code, then accept there',
                ),
              _ => StatsCard(progress: p),
            },
          ],
        ),
        bottom: _finished
            ? FilledButton(onPressed: _close, child: const Text('Done'))
            : TextButton(onPressed: _cancel, child: const Text('Cancel')),
      ),
    );
  }

  String _status(TransferPhase phase) => switch (phase) {
        TransferPhase.connecting => 'Connecting…',
        TransferPhase.waiting => widget.incoming ? 'Starting…' : 'Waiting for ${widget.peerName} to accept',
        TransferPhase.transferring =>
          ['Connected', if (widget.link?.isNotEmpty ?? false) widget.link!, 'Encrypted'].join(' · '),
        TransferPhase.reconnecting => 'Link dropped · Reconnecting…',
        TransferPhase.completed => 'Complete',
        TransferPhase.failed => 'Stopped',
      };
}

class _Outcome extends StatelessWidget {
  const _Outcome({required this.title, required this.detail});

  final String title;
  final String detail;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Column(
      children: [
        Text(title, style: text.titleLarge, textAlign: TextAlign.center),
        const SizedBox(height: Space.xs),
        Text(detail, style: text.bodyMedium?.copyWith(color: AppColors.textSecondary), textAlign: TextAlign.center),
      ],
    );
  }
}
