import 'dart:async';

import '../ui/format.dart';
import 'engine.dart';
import 'models.dart';
import 'platform.g.dart';
import 'rust/api.dart' as rust;

/// The real engine: transfers run in Rust, radios and system services in Kotlin.
class NativeEngine implements TransferEngine, NearvoltEvents {
  /// The phone at the other end of a USB cable, when one is plugged in.
  static const _usb = 'usb';

  final _platform = NearvoltPlatform();
  final _found = <String, NearbyDevice>{};
  final _nearby = StreamController<List<NearbyDevice>>.broadcast();
  final _taps = StreamController<String>.broadcast();
  final _updateProgress = StreamController<double>.broadcast();
  late DeviceInfo _device;
  UpdateInfo? _update;
  String? _incomingFrom;

  /// Sessions share the engine and the radios, so each waits for the previous
  /// one to finish cleaning up; otherwise a closing session could stop the next.
  Future<void> _sendClosed = Future.value();
  Future<void> _receiveClosed = Future.value();

  @override
  Future<DeviceProfile> start() async {
    NearvoltEvents.setUp(this);
    _device = await _platform.deviceInfo();
    rust.startEngine(dataDir: _device.dataDir, deviceName: _device.name);
    return DeviceProfile(
      name: _device.name,
      kind: DeviceKind.parse(_device.kind),
      hasNfc: _device.hasNfc,
      version: _device.versionName,
    );
  }

  @override
  Future<List<Need>> prepare() async {
    var readiness = await _platform.fix(Requirement.permissions);
    if (readiness.permission && !readiness.bluetooth) readiness = await _platform.fix(Requirement.bluetooth);
    return _needs(readiness);
  }

  @override
  Future<List<Need>> needs() async => _needs(await _platform.readiness());

  @override
  Future<List<Need>> fix(Need need) async => _needs(await _platform.fix(switch (need) {
        Need.permission => Requirement.permissions,
        Need.permissionInSettings => Requirement.appSettings,
        Need.wifi => Requirement.wifi,
        Need.bluetooth => Requirement.bluetooth,
        Need.location => Requirement.location,
      }));

  List<Need> _needs(Readiness r) => [
        if (!r.permission) r.permissionInSettings ? Need.permissionInSettings : Need.permission,
        if (!r.wifi) Need.wifi,
        if (!r.bluetooth) Need.bluetooth,
        if (!r.location) Need.location,
      ];

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
      case PlatformEventKind.usbConnected:
        try {
          rust.attachUsb(fd: event.fd!, host: event.host ?? false);
        } on Object catch (e) {
          _platform.log('usb', "the engine couldn't use the cable: $e");
          return;
        }
        _found[_usb] = NearbyDevice(
          id: _usb,
          name: event.name ?? 'Phone on USB',
          kind: DeviceKind.phone,
          link: 'USB cable',
        );
        _publishNearby();
      case PlatformEventKind.usbDisconnected:
        _found.remove(_usb);
        _publishNearby();
    }
  }

  /// One entry per device; the local network wins over Bluetooth because it
  /// already carries the ticket. The cable always gets its own entry.
  void _publishNearby() {
    final byName = <String, NearbyDevice>{};
    for (final device in _found.values) {
      final key = device.id == _usb ? _usb : device.name;
      final existing = byName[key];
      if (existing == null || (existing.ticket == null && device.ticket != null)) byName[key] = device;
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
        _found.removeWhere((id, _) => id != _usb);
        _publishNearby();
        await _platform.watchUsb();
        await _platform.startDiscovery();
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
    final previous = _sendClosed;
    final closed = Completer<void>();
    _sendClosed = closed.future;
    Timer? fallback;
    var joined = false;
    try {
      await previous;
      final ticket = to.id == _usb ? null : to.ticket ?? await _platform.readTicket(to.id);
      final wifi = ticket == null ? null : rust.readTicket(text: ticket).wifi;
      await _platform.startTransferService('Sending to ${to.name}');
      final files = [for (final f in what.files) rust.SourceFile(path: f.path, size: f.size, modifiedMs: f.modifiedMs)];
      _platform.log('send', '${files.length} files, ${what.bytes} bytes to ${to.name} (${to.link})');
      final events = ticket == null
          ? rust.sendUsb(sourceToken: what.token, files: files)
          : rust.sendFiles(ticket: ticket, sourceToken: what.token, files: files);
      // Not on the same network: join the receiver's Wi-Fi Direct group. The engine
      // keeps retrying every address in the ticket, including the group's.
      var connected = false;
      if (wifi != null) {
        fallback = Timer(const Duration(milliseconds: 2500), () async {
          if (!connected) {
            joined = await _platform.joinWifi(WifiGroup(ssid: wifi.ssid, passphrase: wifi.passphrase, frequency: wifi.frequency));
          }
        });
      }
      yield* _track(events, onConnected: () => connected = true);
    } finally {
      try {
        fallback?.cancel();
        rust.stopSending();
        await _platform.stopTransferService();
        if (joined) await _platform.leaveWifi();
      } finally {
        closed.complete();
      }
    }
  }

  @override
  Stream<ReceiveEvent> receive() async* {
    final previous = _receiveClosed;
    final closed = Completer<void>();
    _receiveClosed = closed.future;
    final tracker = _Tracker();
    try {
      await previous;
      await _platform.watchUsb();
      final events = rust.receive(destDir: _device.downloadDir, journalDir: _device.journalDir);
      await for (final event in events) {
        _record('receive', event);
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
      try {
        rust.stopReceiving();
        await _platform.stopReceiving();
        await _platform.stopTransferService();
      } finally {
        closed.complete();
      }
    }
  }

  @override
  void answer({required bool accept}) {
    rust.answerOffer(accept: accept);
    _platform.log('receive', accept ? 'accepted' : 'declined');
    if (!accept) return;
    // Connected: the radios that found this phone have done their job.
    _platform.stopAdvertising();
    _platform.startTransferService('Receiving from ${_incomingFrom ?? 'a nearby device'}');
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

  /// The milestones of a transfer, for the log; progress ticks are left out.
  void _record(String tag, rust.TransferEvent e) {
    final line = switch (e.kind) {
      rust.EventKind.connected => 'connected to ${e.text}',
      rust.EventKind.code => 'showing a confirmation code',
      rust.EventKind.offer => 'offer of ${e.totalFiles} files, ${e.totalBytes} bytes from ${e.text}',
      rust.EventKind.connectionLost => 'connection lost: ${e.text}',
      rust.EventKind.completed => 'completed: ${e.doneFiles} files, ${e.doneBytes} bytes',
      rust.EventKind.failed => 'failed after ${e.doneBytes} bytes: ${e.text}',
      rust.EventKind.ready || rust.EventKind.progress => null,
    };
    if (line != null) _platform.log(tag, line);
  }

  Stream<TransferProgress> _track(Stream<rust.TransferEvent> events, {void Function()? onConnected}) async* {
    final tracker = _Tracker();
    await for (final event in events) {
      _record('send', event);
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
