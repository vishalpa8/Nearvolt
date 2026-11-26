import 'dart:async';

import '../ui/format.dart';
import 'engine.dart';
import 'models.dart';
import 'platform.g.dart';
import 'rust/api.dart' as rust;

/// The real engine: transfers run in Rust, radios and system services in Kotlin.
class NativeEngine implements TransferEngine, NearvoltEvents {
  final _platform = NearvoltPlatform();
  final _found = <String, NearbyDevice>{};
  final _nearby = StreamController<List<NearbyDevice>>.broadcast();
  final _taps = StreamController<String>.broadcast();
  final _updateProgress = StreamController<double>.broadcast();
  late DeviceInfo _device;
  UpdateInfo? _update;
  String? _incomingFrom;

  @override
  Future<DeviceProfile> start() async {
    NearvoltEvents.setUp(this);
    _device = await _platform.deviceInfo();
    rust.startEngine(dataDir: _device.dataDir, deviceName: _device.name);
    return DeviceProfile(name: _device.name, kind: DeviceKind.parse(_device.kind), hasNfc: _device.hasNfc);
  }

  @override
  void onEvent(PlatformEvent event) {
    switch (event.kind) {
      case PlatformEventKind.receiverFound:
        _found[event.id!] = NearbyDevice(
          id: event.id!,
          name: event.name ?? 'Nearby device',
          kind: DeviceKind.parse(event.deviceKind),
          link: event.link ?? '',
          ticket: event.ticket,
        );
        _publishNearby();
      case PlatformEventKind.receiverLost:
        _found.remove(event.id);
        _publishNearby();
      case PlatformEventKind.ticketTapped:
        if (event.ticket case final ticket?) _taps.add(ticket);
      case PlatformEventKind.updateProgress:
        _updateProgress.add(event.progress ?? 0);
      case PlatformEventKind.usbConnected || PlatformEventKind.usbDisconnected:
        break;
    }
  }

  /// One entry per device; the local network wins over Bluetooth because it
  /// already carries the ticket.
  void _publishNearby() {
    final byName = <String, NearbyDevice>{};
    for (final device in _found.values) {
      final existing = byName[device.name];
      if (existing == null || (existing.ticket == null && device.ticket != null)) byName[device.name] = device;
    }
    _nearby.add(byName.values.toList());
  }

  @override
  Stream<List<NearbyDevice>> nearby() {
    late final StreamController<List<NearbyDevice>> controller;
    StreamSubscription<List<NearbyDevice>>? forward;
    controller = StreamController(
      onListen: () async {
        forward = _nearby.stream.listen(controller.add);
        _found.clear();
        controller.add(const []);
        if (await _platform.requestPermissions(['nearby'])) await _platform.startDiscovery();
      },
      onCancel: () async {
        await forward?.cancel();
        await _platform.stopDiscovery();
      },
    );
    return controller.stream;
  }

  @override
  Stream<String> taps() => _taps.stream;

  @override
  NearbyDevice deviceFor(String ticket) {
    final info = rust.readTicket(text: ticket);
    return NearbyDevice(id: 'code:${info.name}', name: info.name, kind: DeviceKind.phone, link: 'Code', ticket: ticket);
  }

  @override
  Future<Selection?> pickFiles() async => _selection(await _platform.pickFiles());

  @override
  Future<Selection?> pickFolder() async => _selection(await _platform.pickFolder());

  Selection? _selection(PickResult? result) => result == null
      ? null
      : Selection(
          token: result.token,
          files: [for (final f in result.files) SelectedFile(path: f.path, size: f.size, modifiedMs: f.modifiedMs)],
        );

  @override
  Future<String?> scanCode() => _platform.scanQr();

  @override
  Stream<TransferProgress> send(NearbyDevice to, Selection what) async* {
    yield const TransferProgress(phase: TransferPhase.connecting);
    final ticket = to.ticket ?? await _platform.readTicket(to.id);
    final wifi = rust.readTicket(text: ticket).wifi;
    await _platform.requestPermissions(['notifications']);
    await _platform.startTransferService('Sending to ${to.name}');
    final events = rust.sendFiles(
      ticket: ticket,
      sourceToken: what.token,
      files: [for (final f in what.files) rust.SourceFile(path: f.path, size: f.size, modifiedMs: f.modifiedMs)],
    );
    // Not on the same network: join the receiver's Wi-Fi Direct group. The engine
    // keeps retrying every address in the ticket, including the group's.
    var connected = false;
    var joined = false;
    final fallback = wifi == null
        ? null
        : Timer(const Duration(milliseconds: 2500), () async {
            if (!connected) joined = await _platform.joinWifi(WifiGroup(ssid: wifi.ssid, passphrase: wifi.passphrase, frequency: wifi.frequency));
          });
    try {
      yield* _track(events, onConnected: () => connected = true);
    } finally {
      fallback?.cancel();
      rust.stopSending();
      await _platform.stopTransferService();
      if (joined) await _platform.leaveWifi();
    }
  }

  @override
  Stream<ReceiveEvent> receive() async* {
    await _platform.requestPermissions(['nearby', 'notifications']);
    final events = rust.receive(destDir: _device.downloadDir, journalDir: _device.journalDir);
    final tracker = _Tracker();
    try {
      await for (final event in events) {
        switch (event.kind) {
          case rust.EventKind.ready:
            var tickets = rust.receiveTickets();
            yield ReceiveReady(tickets.privateTicket);
            final group = await _platform.startReceiving(tickets.privateTicket, tickets.publicTicket, tickets.port);
            if (group != null) {
              final wifi = rust.WifiInfo(ssid: group.ssid, passphrase: group.passphrase, frequency: group.frequency);
              tickets = rust.receiveTickets(wifi: wifi);
              await _platform.updateTickets(tickets.privateTicket, tickets.publicTicket, tickets.port);
              yield ReceiveReady(tickets.privateTicket);
            }
          case rust.EventKind.offer:
            _incomingFrom = event.text ?? 'Someone nearby';
            yield ReceiveOffer(IncomingOffer(
              from: NearbyDevice(id: 'sender', name: _incomingFrom!, kind: DeviceKind.phone, link: 'Wi-Fi'),
              files: event.totalFiles,
              bytes: event.totalBytes,
              code: event.code,
            ));
          default:
            final progress = tracker.apply(event);
            if (progress != null) {
              _notify(progress);
              yield ReceiveProgress(progress);
            }
        }
      }
    } finally {
      rust.stopReceiving();
      await _platform.stopReceiving();
      await _platform.stopTransferService();
    }
  }

  @override
  void answer({required bool accept}) {
    rust.answerOffer(accept: accept);
    if (accept) _platform.startTransferService('Receiving from ${_incomingFrom ?? 'a nearby device'}');
  }

  @override
  void stopTransfer() {
    rust.stopSending();
    rust.stopReceiving();
  }

  @override
  Future<String?> checkForUpdate() async {
    _update = await _platform.checkForUpdate();
    return _update?.version;
  }

  @override
  Stream<double> installUpdate() {
    final controller = StreamController<double>();
    final forward = _updateProgress.stream.listen(controller.add);
    _platform.installUpdate(_update!).then(
      (_) => controller.close(),
      onError: (Object e) {
        controller.addError(e);
        controller.close();
      },
    ).whenComplete(forward.cancel);
    return controller.stream;
  }

  Stream<TransferProgress> _track(Stream<rust.TransferEvent> events, {void Function()? onConnected}) async* {
    final tracker = _Tracker();
    await for (final event in events) {
      if (event.kind == rust.EventKind.connected) onConnected?.call();
      final progress = tracker.apply(event);
      if (progress != null) {
        _notify(progress);
        yield progress;
      }
    }
  }

  DateTime _notified = DateTime(0);

  /// The notification needs no more than one update a second.
  void _notify(TransferProgress p) {
    final now = DateTime.now();
    if (p.finished || now.difference(_notified) < const Duration(seconds: 1)) return;
    _notified = now;
    _platform.updateTransferService('${formatBytes(p.doneBytes)} of ${formatBytes(p.totalBytes)}', (p.fraction * 100).round());
  }
}

/// Turns engine events into the progress the UI shows, with a smoothed speed.
class _Tracker {
  var _phase = TransferPhase.connecting;
  String? _code;
  var _last = const TransferProgress(phase: TransferPhase.connecting);
  DateTime? _speedAt;
  var _speedBytes = 0;
  var _speed = 0.0;

  TransferProgress? apply(rust.TransferEvent e) {
    switch (e.kind) {
      case rust.EventKind.connected:
        _phase = _last.doneBytes > 0 ? TransferPhase.transferring : TransferPhase.waiting;
        return _emit(_last.doneBytes, e);
      case rust.EventKind.code:
        _code = e.code;
        _phase = TransferPhase.waiting;
        return _emit(_last.doneBytes, e);
      case rust.EventKind.connectionLost:
        _phase = TransferPhase.reconnecting;
        _speed = 0;
        return _emit(_last.doneBytes, e);
      case rust.EventKind.progress:
        if (_phase == TransferPhase.waiting && e.doneBytes > 0) _phase = TransferPhase.transferring;
        if (_phase == TransferPhase.connecting && e.doneBytes > 0) _phase = TransferPhase.transferring;
        return _emit(e.doneBytes, e);
      case rust.EventKind.completed:
        _phase = TransferPhase.completed;
        return _emit(e.totalBytes, e);
      case rust.EventKind.failed:
        _phase = TransferPhase.failed;
        return _emit(e.doneBytes, e, error: e.text);
      case rust.EventKind.ready || rust.EventKind.offer:
        return null;
    }
  }

  TransferProgress _emit(int doneBytes, rust.TransferEvent e, {String? error}) {
    final now = DateTime.now();
    final at = _speedAt;
    if (at == null || _phase != TransferPhase.transferring) {
      _speedAt = now;
      _speedBytes = doneBytes;
    } else if (now.difference(at) >= const Duration(milliseconds: 500)) {
      final instant = (doneBytes - _speedBytes) / (now.difference(at).inMicroseconds / 1e6);
      _speed = _speed == 0 ? instant : _speed * 0.7 + instant * 0.3;
      _speedAt = now;
      _speedBytes = doneBytes;
    }
    _last = TransferProgress(
      phase: _phase,
      doneBytes: doneBytes,
      totalBytes: e.totalBytes > 0 ? e.totalBytes : _last.totalBytes,
      doneFiles: e.doneFiles > 0 ? e.doneFiles : _last.doneFiles,
      totalFiles: e.totalFiles > 0 ? e.totalFiles : _last.totalFiles,
      bytesPerSecond: _phase == TransferPhase.transferring ? _speed.clamp(0, double.infinity) : 0,
      code: _phase == TransferPhase.waiting ? _code : null,
      error: error,
    );
    return _last;
  }
}
