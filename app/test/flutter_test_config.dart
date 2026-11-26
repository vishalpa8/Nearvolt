import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// Loads the real fonts so golden screenshots show text and icons, not boxes.
Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  TestWidgetsFlutterBinding.ensureInitialized();
  final flutterRoot = Platform.environment['FLUTTER_ROOT'] ?? '../.toolchain/flutter';
  await _load('Inter', ['Regular', 'Medium', 'SemiBold'].map((w) => 'assets/fonts/Inter-$w.ttf'));
  await _load('MaterialIcons', ['$flutterRoot/bin/cache/artifacts/material_fonts/MaterialIcons-Regular.otf']);
  await testMain();
}

Future<void> _load(String family, Iterable<String> paths) async {
  final loader = FontLoader(family);
  for (final path in paths) {
    loader.addFont(File(path).readAsBytes().then(ByteData.sublistView));
  }
  await loader.load();
}
