import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/messages.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/atoms.dart';
import '../ui/format.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';

/// One open connection, on either phone: how it's doing, the batch on the move
/// and every batch so far. The sender adds files here; either phone disconnects.
class ConnectionPage extends StatefulWidget {
  const ConnectionPage({
    super.key,
    required this.engine,
    required this.updates,
    required this.initial,
    required this.sending,
    this.files,
  });

  final TransferEngine engine;
  final Stream<Connection> updates;
  final Connection initial;
  final bool sending;

  /// Chosen before connecting: sent as soon as the connection is fully set up.
  final Selection? files;

  @override
  State<ConnectionPage> createState() => _ConnectionPageState();
}

class _ConnectionPageState extends State<ConnectionPage> {
  late Connection _c = widget.initial;
  late final StreamSubscription<Connection> _sub;
  var _leaving = false;

  /// The engine has already been told to end this connection.
  var _disconnected = false;
  late var _pending = widget.files;

  /// Phones paired by a code (Nearby list, USB) must have it compared on this screen
  /// too, since nothing else proves the phone that answered is the one picked.
  var _codeChecked = false;

  bool _needsCodeCheck(Connection c) =>
      widget.sending && c.phase == LinkPhase.connected && c.code != null && !_codeChecked;

  @override
  void initState() {
    super.initState();
    _sub = widget.updates.listen(_update);
    _update(_c);
  }

  void _update(Connection c) {
    // Files chosen first wait for the connection to be fully set up, and for the code to
    // be checked if there is one.
    if (c.phase == LinkPhase.connected && !_needsCodeCheck(c)) _sendPending();
    if (mounted) setState(() => _c = c);
  }

  void _sendPending() {
    final files = _pending;
    if (files == null) return;
    _pending = null;
    try {
      widget.engine.queue(files);
    } on Object {
      _tell(connectionEnded);
    }
  }

  void _tell(String message) {
    if (mounted) ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(message)));
  }

  void _codeMatches() {
    setState(() => _codeChecked = true);
    _sendPending();
  }

  void _codeDiffers() {
    setState(() => _leaving = true);
    widget.engine.disconnect();
    _tell("The codes differ, so this may not be ${_c.peer}. Nothing was sent.");
  }

  @override
  void dispose() {
    _sub.cancel();
    super.dispose();
  }

  Future<void> _add() async {
    final kind = await showPickMenu(context);
    if (kind == null) return;
    final engine = widget.engine;
    final selection = kind == PickKind.folder ? await engine.pickFolder() : await engine.pickFiles();
    if (selection == null) return;
    if (selection.skipped > 0) _tell(leftOut(selection.skipped));
    if (selection.files.isEmpty) return;
    try {
      engine.queue(selection);
    } on Object {
      _tell(connectionEnded);
    }
  }

  /// Before the other phone has accepted there's nothing to disconnect from, so
  /// leaving just cancels; only a connection that was set up asks first.
  Future<void> _disconnect() async {
    if (_c.phase != LinkPhase.connected) {
      // The screen goes at once; the engine finishes cleaning up in the background,
      // even if it's still in the middle of joining a network.
      _disconnected = true;
      widget.engine.disconnect();
      if (mounted) Navigator.of(context).pop();
      return;
    }
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Disconnect from ${_c.peer}?'),
        content: const Text('Files that have finished arriving stay. A file still arriving is removed.'),
        actions: [
          TextButton(onPressed: () => Navigator.of(context).pop(false), child: const Text('Stay connected')),
          TextButton(onPressed: () => Navigator.of(context).pop(true), child: const Text('Disconnect')),
        ],
      ),
    );
    if (sure != true || !mounted) return;
    setState(() => _leaving = true);
    widget.engine.disconnect();
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final c = _c;
    final (status, color) = switch (c.phase) {
      LinkPhase.connecting ||
      LinkPhase.waiting ||
      LinkPhase.reconnecting when _leaving => ('Cancelling…', AppColors.warning),
      LinkPhase.connecting => ('Connecting…', AppColors.accent),
      LinkPhase.waiting => ('Waiting for ${c.peer} to accept', AppColors.accent),
      LinkPhase.connected when _leaving => ('Disconnecting…', AppColors.warning),
      LinkPhase.connected when _needsCodeCheck(c) => ('Connected · check the code', AppColors.accent),
      LinkPhase.connected => (['Connected', if (c.link.isNotEmpty) c.link, 'Encrypted'].join(' · '), AppColors.success),
      LinkPhase.reconnecting => ('Link dropped · Reconnecting…', AppColors.warning),
      LinkPhase.ended => (c.endedByPeer ? '${c.peer} disconnected' : 'Disconnected', AppColors.textSecondary),
      LinkPhase.failed => ('Connection failed', AppColors.danger),
    };
    return PopScope(
      // Only a connection that was set up asks first; before that, Back just cancels.
      canPop: !c.open || c.phase != LinkPhase.connected,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) {
          _disconnect();
        } else if (c.open && !_disconnected) {
          widget.engine.disconnect();
        }
      },
      child: PageScaffold(
        body: ListView(
          padding: const EdgeInsets.only(top: Space.l, bottom: Space.xl),
          children: [
            Text(
              widget.sending ? 'Sending to' : 'Receiving from',
              style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
            ),
            const SizedBox(height: 2),
            Text(c.peer, style: text.titleLarge),
            const SizedBox(height: Space.m),
            Align(
              alignment: Alignment.centerLeft,
              child: Pill(label: status, color: color),
            ),
            const SizedBox(height: Space.xxl),
            _middle(context, c),
            if (c.batches.isNotEmpty) ...[
              const SizedBox(height: Space.xxl),
              Text(widget.sending ? 'Sent on this connection' : 'Received on this connection', style: text.titleMedium),
              const SizedBox(height: Space.xs),
              BatchList(batches: c.batches, sending: widget.sending),
            ],
          ],
        ),
        bottom: !c.open
            ? FilledButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Done'))
            : Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  // Files can be added once the connection is set up, not before.
                  if (widget.sending && c.phase == LinkPhase.connected && !_needsCodeCheck(c))
                    FilledButton.icon(
                      onPressed: _leaving ? null : _add,
                      icon: const Icon(Icons.add_rounded),
                      label: const Text('Add files'),
                    ),
                  const SizedBox(height: Space.s),
                  TextButton(
                    onPressed: _leaving ? null : _disconnect,
                    child: Text(switch (c.phase) {
                      LinkPhase.connected => 'Disconnect',
                      LinkPhase.reconnecting => 'Stop reconnecting',
                      _ => 'Cancel',
                    }),
                  ),
                ],
              ),
      ),
    );
  }

  Widget _middle(BuildContext context, Connection c) {
    final text = Theme.of(context).textTheme;
    final p = c.progress;
    if (c.phase == LinkPhase.waiting && c.code != null) {
      return CodeBadge(code: c.code!, caption: 'Check that ${c.peer} shows the same code, then accept there');
    }
    if (_needsCodeCheck(c) && !_leaving) {
      return Column(
        children: [
          CodeBadge(code: c.code!, caption: 'Does ${c.peer} show this same code?'),
          const SizedBox(height: Space.l),
          Text(
            'If the codes differ, another phone may be pretending to be ${c.peer}.',
            style: text.bodySmall,
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: Space.l),
          Row(
            children: [
              Expanded(
                child: OutlinedButton(onPressed: _codeDiffers, child: const Text("It doesn't match")),
              ),
              const SizedBox(width: Space.m),
              Expanded(
                child: FilledButton(onPressed: _codeMatches, child: const Text('It matches')),
              ),
            ],
          ),
        ],
      );
    }
    if (p != null && c.open) {
      return Column(
        children: [
          ProgressRing(
            fraction: p.fraction,
            color: c.phase == LinkPhase.reconnecting ? AppColors.warning : AppColors.accent,
            size: 200,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  '${(p.fraction * 100).floor()}%',
                  style: text.displaySmall?.copyWith(fontFeatures: tabularFigures),
                ),
                const SizedBox(height: Space.xs),
                Text(
                  '${formatBytes(p.doneBytes)} of ${formatBytes(p.totalBytes)}',
                  style: text.bodySmall?.copyWith(fontFeatures: tabularFigures),
                ),
              ],
            ),
          ),
          const SizedBox(height: Space.xl),
          StatsCard(progress: p),
        ],
      );
    }
    final (icon, line) = switch (c.phase) {
      LinkPhase.connecting || LinkPhase.waiting => (Icons.link_rounded, 'Setting up a direct, encrypted link.'),
      LinkPhase.reconnecting => (Icons.sync_rounded, 'Nothing is lost: it picks up where it stopped.'),
      LinkPhase.ended => (Icons.link_off_rounded, 'Everything that finished was verified and saved.'),
      LinkPhase.failed => (Icons.error_outline_rounded, c.error ?? friendlyFailure(null)),
      LinkPhase.connected when widget.sending => (
        Icons.add_circle_outline_rounded,
        'Add files to send them to ${c.peer}.',
      ),
      LinkPhase.connected => (Icons.download_rounded, 'Files ${c.peer} sends appear here.'),
    };
    return Panel(
      child: Row(
        children: [
          Icon(icon, color: AppColors.textSecondary),
          const SizedBox(width: Space.m),
          Expanded(child: Text(line, style: text.bodyMedium)),
        ],
      ),
    );
  }
}
