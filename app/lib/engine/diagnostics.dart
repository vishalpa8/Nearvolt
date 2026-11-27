import 'dart:async';

import '../ui/format.dart';
import 'platform.g.dart';
import 'rust/api.dart' as rust;

/// Writes the log lines that explain one connection: how it was made and over
/// which route, the network and the phone's condition, and while files move a
/// speed sample every few seconds, so a slow or dropped transfer can be traced
/// to its cause. Progress ticks themselves are left out.
class Diagnostics {
  Diagnostics(this._platform, this._tag, {required bool sending, this.sampleEvery = const Duration(seconds: 5)})
    : _connecting = sending ? DateTime.now() : null;

  final NearvoltPlatform _platform;
  final String _tag;
  final Duration sampleEvery;

  /// Sender: when connecting began, until the first connection.
  DateTime? _connecting;
  DateTime? _lostAt;

  Timer? _sampler;
  int? _batch;
  DateTime? _batchStarted;
  ({DateTime at, int bytes})? _last;
  double? _fastest;
  double? _slowest;
  var _done = 0;
  var _total = 0;

  /// The network and the phone as they are now, on one line.
  Future<void> network() async => _log('network', await _report());

  void record(rust.TransferEvent e) {
    switch (e.kind) {
      case rust.EventKind.request:
        _log(_tag, 'a phone is asking to connect');
      case rust.EventKind.code:
        _log(_tag, 'showing a confirmation code');
      case rust.EventKind.connected:
        _connected(e);
      case rust.EventKind.started:
        _started(e);
      case rust.EventKind.progress:
        _done = e.doneBytes;
        _total = e.totalBytes;
        // Each batch, and each reconnect, measures from its own first progress.
        if (_sampler != null) _last ??= (at: DateTime.now(), bytes: e.doneBytes);
      case rust.EventKind.completed:
        _log(
          _tag,
          'batch ${e.batch} completed: ${e.totalFiles} files, ${formatBytes(e.totalBytes)}${_summary(e.totalBytes)}',
        );
        _endBatch();
      case rust.EventKind.batchFailed:
        _log(_tag, 'batch ${e.batch} failed (${e.failure?.name}): ${e.text}');
        _endBatch();
      case rust.EventKind.connectionLost:
        _lostAt = DateTime.now();
        _log(_tag, 'connection lost: ${e.text}');
        network();
      case rust.EventKind.peerLeft:
        _log(_tag, 'the other phone disconnected');
      case rust.EventKind.disconnected:
        _log(_tag, 'disconnected after ${e.doneFiles} files, ${formatBytes(e.doneBytes)}');
        close();
      case rust.EventKind.failed:
        _log(_tag, 'failed (${e.failure?.name}): ${e.text}');
        close();
      case rust.EventKind.requestWithdrawn:
        _log(_tag, 'the phone asking to connect left before an answer');
      case rust.EventKind.note:
        _log(_tag, e.text ?? '');
      case rust.EventKind.ready:
        break;
    }
  }

  void close() {
    _sampler?.cancel();
    _sampler = null;
  }

  void _connected(rust.TransferEvent e) {
    final now = DateTime.now();
    final route = e.route;
    final via = route == null ? '' : (route == 'USB cable' ? ' over the USB cable' : ' at $route (${_kind(route)})');
    final (connecting, lost) = (_connecting, _lostAt);
    final after = connecting != null
        ? ' in ${_seconds(now.difference(connecting))}'
        : lost != null
        ? ' again, ${_seconds(now.difference(lost))} after the link dropped'
        : '';
    _connecting = null;
    _lostAt = null;
    _log(_tag, 'connected to the other phone$via$after');
    network();
  }

  /// The receiver's Wi-Fi Direct group always has this address; any other is a
  /// network both phones share.
  static String _kind(String route) => route.startsWith('192.168.49.') ? 'Wi-Fi Direct' : 'same Wi-Fi';

  void _started(rust.TransferEvent e) {
    // A batch resumed after a reconnect is the same batch, so its clock keeps running.
    if (_batch != e.batch) {
      _batch = e.batch;
      _batchStarted = DateTime.now();
      _fastest = null;
      _slowest = null;
      _log(_tag, 'batch ${e.batch}: ${e.totalFiles} files, ${formatBytes(e.totalBytes)}');
    }
    _last = null;
    _sampler ??= Timer.periodic(sampleEvery, (_) => _sample());
  }

  Future<void> _sample() async {
    final now = DateTime.now();
    final last = _last;
    if (last == null) return;
    _last = (at: now, bytes: _done);
    final seconds = now.difference(last.at).inMilliseconds / 1000;
    if (seconds <= 0) return;
    final speed = (_done - last.bytes).clamp(0, 1 << 62) / seconds;
    _fastest = speed > (_fastest ?? -1) ? speed : _fastest;
    _slowest = speed < (_slowest ?? double.infinity) ? speed : _slowest;
    final percent = _total == 0 ? 0 : (_done * 100 / _total).floor();
    final report = await _report();
    _log(
      'speed',
      'batch $_batch: $percent% (${formatBytes(_done)} of ${formatBytes(_total)}), '
          '${formatBytes(speed.round())}/s over the last ${seconds.round()} s · $report',
    );
  }

  String _summary(int bytes) {
    final started = _batchStarted;
    if (started == null) return '';
    final seconds = DateTime.now().difference(started).inMilliseconds / 1000;
    if (seconds <= 0) return '';
    final (fastest, slowest) = (_fastest, _slowest);
    final range = fastest == null || slowest == null
        ? ''
        : '; fastest ${formatBytes(fastest.round())}/s, slowest ${formatBytes(slowest.round())}/s';
    return ' in ${_seconds(Duration(milliseconds: (seconds * 1000).round()))}, '
        'average ${formatBytes((bytes / seconds).round())}/s$range';
  }

  void _endBatch() {
    close();
    _batch = null;
    _batchStarted = null;
  }

  Future<String> _report() async {
    try {
      return await _platform.linkReport();
    } on Object catch (e) {
      return 'network unknown ($e)';
    }
  }

  static String _seconds(Duration d) => '${(d.inMilliseconds / 1000).toStringAsFixed(1)} s';

  void _log(String tag, String line) => _platform.log(tag, line);
}
