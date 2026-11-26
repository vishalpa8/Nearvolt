import 'dart:math';

/// Decimal units, for example `312 GB`.
String formatBytes(int bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  if (unit == 0) return '$bytes B';
  final digits = value >= 100 ? 0 : (value >= 10 ? 1 : 2);
  return '${value.toStringAsFixed(digits)} ${units[unit]}';
}

String formatSpeed(double bytesPerSecond) => '${formatBytes(bytesPerSecond.round())}/s';

/// Rounds up, so the estimate never promises less time than remains.
String formatDuration(Duration d) {
  final seconds = max(d.inSeconds, 1);
  if (seconds < 60) return '$seconds s';
  final minutes = (seconds / 60).ceil();
  if (minutes < 60) return '$minutes min';
  return '${minutes ~/ 60} h ${minutes % 60} min';
}

/// `1903` becomes `1,903`.
String formatCount(int n) {
  return n.toString().replaceAllMapped(RegExp(r'(\d)(?=(\d{3})+$)'), (m) => '${m[1]},');
}

String filesLabel(int n) => n == 1 ? '1 file' : '${formatCount(n)} files';
