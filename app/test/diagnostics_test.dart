import 'package:flutter_test/flutter_test.dart';
import 'package:nearvolt/engine/diagnostics.dart';
import 'package:nearvolt/engine/platform.g.dart';
import 'package:nearvolt/engine/rust/api.dart';

class _Platform extends NearvoltPlatform {
  final lines = <String>[];

  @override
  Future<void> log(String tag, String message) async => lines.add('[$tag] $message');

  @override
  Future<String> linkReport() async => 'network: Wi-Fi 6, 5 GHz (5180 MHz), signal -50 dBm';
}

TransferEvent _event(EventKind kind, {String? text, String? route, int batch = -1, int done = 0, int total = 0}) =>
    TransferEvent(
      kind: kind,
      text: text,
      route: route,
      batch: batch,
      doneBytes: done,
      totalBytes: total,
      doneFiles: 0,
      totalFiles: 1,
    );

void main() {
  test('logs the route, the network, a speed sample while files move, and the batch summary', () async {
    final platform = _Platform();
    final diagnostics = Diagnostics(platform, 'send', sending: true, sampleEvery: const Duration(milliseconds: 200));

    diagnostics.record(_event(EventKind.connected, text: 'Pixel 9', route: '192.168.49.1:40599'));
    diagnostics.record(_event(EventKind.started, batch: 0, total: 100000000));
    diagnostics.record(_event(EventKind.progress, batch: 0, done: 0, total: 100000000));
    await Future<void>.delayed(const Duration(milliseconds: 100));
    diagnostics.record(_event(EventKind.progress, batch: 0, done: 40000000, total: 100000000));
    await Future<void>.delayed(const Duration(milliseconds: 250));
    diagnostics.record(_event(EventKind.completed, batch: 0, total: 100000000));
    await Future<void>.delayed(Duration.zero);

    final log = platform.lines.join('\n');
    expect(log, contains('[send] connected to Pixel 9 at 192.168.49.1:40599 (Wi-Fi Direct) in '));
    expect(log, contains('[network] network: Wi-Fi 6'));
    expect(log, contains('[speed] batch 0: 40% (40.0 MB of 100 MB), '));
    expect(log, contains('· network: Wi-Fi 6'));
    expect(log, matches(RegExp(r'batch 0 completed: .* average .*/s; fastest .*/s, slowest .*/s')));

    // Sampling stops with the batch.
    final count = platform.lines.length;
    await Future<void>.delayed(const Duration(milliseconds: 450));
    expect(platform.lines.length, count);
  });

  test('a reconnect says how long the link was down', () async {
    final platform = _Platform();
    final diagnostics = Diagnostics(platform, 'send', sending: true);

    diagnostics.record(_event(EventKind.connected, text: 'Pixel 9', route: '10.0.0.7:40599'));
    diagnostics.record(_event(EventKind.connectionLost, text: 'no word from the receiver for 4 seconds'));
    diagnostics.record(_event(EventKind.connected, text: 'Pixel 9', route: '10.0.0.7:40599'));
    await Future<void>.delayed(Duration.zero);

    expect(platform.lines, contains(startsWith('[send] connected to Pixel 9 at 10.0.0.7:40599 (same Wi-Fi) in ')));
    expect(platform.lines, contains('[send] connection lost: no word from the receiver for 4 seconds'));
    expect(platform.lines, contains(contains('again, ')));
    diagnostics.close();
  });
}
