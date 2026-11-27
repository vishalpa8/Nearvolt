import 'dart:async';

import 'package:flutter/foundation.dart' show ValueListenable, ValueNotifier;

import '../ui/format.dart';
import 'diagnostics.dart';
import 'engine.dart';
import 'messages.dart';
import 'models.dart';
import 'platform.g.dart';
import 'rust/api.dart' as rust;

/// The real engine: transfers run in Rust, radios and system services in Kotlin.
class NativeEngine implements TransferEngine, NearvoltEvents {
  final _platform = NearvoltPlatform();
  final _found = <String, NearbyDevice>{};
  final _nearby = StreamController<List<NearbyDevice>>.broadcast();
  final _taps = StreamController<String>.broadcast();
  final _usb = StreamController<NearbyDevice?>.broadcast();
  final _updateProgress = StreamController<double>.broadcast();
  late DeviceInfo _device;
  NearbyDevice? _usbPeer;
  UpdateInfo? _update;
  _Live? _sending;

  /// Connections share the engine and the radios, so each waits for the
  /// previous one to finish cleaning up; otherwise a closing one could stop the next.
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
  Future<List<Need>> prepare(Purpose purpose) async {
    var readiness = await _platform.readiness();
    if (purpose.findsPhones) {
      readiness = await _platform.fix(Requirement.permissions);
      if (readiness.permission && !readiness.bluetooth) readiness = await _platform.fix(Requirement.bluetooth);
    }
    return _needs(readiness, purpose);
  }

  @override
  Future<List<Need>> needs(Purpose purpose) async => _needs(await _platform.readiness(), purpose);

  @override
  Future<void> fix(Need need) => _platform.fix(switch (need) {
    Need.permission => Requirement.permissions,
    Need.permissionInSettings => Requirement.appSettings,
    Need.nfc => Requirement.nfc,
    Need.wifi => Requirement.wifi,
    Need.bluetooth => Requirement.bluetooth,
    Need.location => Requirement.location,
  });

  List<Need> _needs(Readiness r, Purpose purpose) => [
    if (purpose.findsPhones && !r.permission) r.permissionInSettings ? Need.permissionInSettings : Need.permission,
    // Receive offers tapping too, so its phone needs NFC on to be tapped.
    if ((purpose == Purpose.tap || purpose == Purpose.receive) && !r.nfc) Need.nfc,
    if (purpose != Purpose.usb && !r.wifi) Need.wifi,
    if (purpose.findsPhones && !r.bluetooth) Need.bluetooth,
    if (purpose.findsPhones && !r.location) Need.location,
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
        if (event.ticket case final ticket?) {
          _platform.log('nfc', 'read a code by tapping phones');
          _taps.add(ticket);
        }
      case PlatformEventKind.updateProgress:
        _updateProgress.add(event.progress ?? 0);
      case PlatformEventKind.usbConnected:
        try {
          rust.attachUsb(fd: event.fd!, host: event.host ?? false);
        } on Object catch (e) {
          _platform.log('usb', "the engine couldn't use the cable: $e");
          return;
        }
        _usb.add(
          _usbPeer = NearbyDevice(
            id: 'usb',
            name: event.name ?? 'Phone on USB',
            kind: DeviceKind.phone,
            link: 'USB cable',
          ),
        );
      case PlatformEventKind.usbDisconnected:
        _usb.add(_usbPeer = null);
      case PlatformEventKind.searchFailed:
        _platform.log('nearby', 'the search of the local network could not start');
        _searchFailed.value = true;
    }
  }

  final _searchFailed = ValueNotifier<bool>(false);

  @override
  ValueListenable<bool> get searchFailed => _searchFailed;

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
  Stream<List<NearbyDevice>> nearby() => _whileListened(
    _nearby.stream,
    start: () {
      _found.clear();
      _searchFailed.value = false;
      return _platform.startNearby();
    },
    stop: _platform.stopNearby,
    initial: [const []],
  );

  @override
  Stream<String> taps() =>
      _whileListened(_taps.stream, start: _platform.startTapReading, stop: _platform.stopTapReading);

  @override
  Stream<NearbyDevice?> usbPeer() =>
      _whileListened(_usb.stream, start: _platform.watchUsb, stop: () async {}, initial: [_usbPeer]);

  /// Runs `start` while someone listens, and `stop` once they stop. `initial`
  /// goes to the listener first.
  Stream<T> _whileListened<T>(
    Stream<T> source, {
    required Future<void> Function() start,
    required Future<void> Function() stop,
    List<T> initial = const [],
  }) {
    late final StreamController<T> controller;
    StreamSubscription<T>? forward;
    controller = StreamController(
      onListen: () async {
        forward = source.listen(controller.add);
        initial.forEach(controller.add);
        await start();
      },
      onCancel: () async {
        await forward?.cancel();
        await stop();
      },
    );
    return controller.stream;
  }

  @override
  NearbyDevice deviceFor(String ticket) {
    final info = rust.readTicket(text: ticket);
    return NearbyDevice(
      id: 'code:${info.name}',
      name: info.name,
      kind: DeviceKind.phone,
      link: 'Wi-Fi',
      ticket: ticket,
    );
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
          skipped: result.skipped,
        );

  @override
  Future<String?> scanCode() async {
    final ticket = await _platform.scanQr();
    if (ticket != null) _platform.log('scanner', 'read a code');
    return ticket;
  }

  @override
  Stream<Connection> connect(NearbyDevice to) {
    final out = StreamController<Connection>();
    final live = _sending = _Live(
      Connection(peer: to.name, link: to.link),
      onChange: out.add,
      receiving: false,
      onRelease: _platform.releaseSelection,
    );
    final previous = _sendClosed;
    final closed = Completer<void>();
    _sendClosed = closed.future;

    var cancelled = false;

    Future<void> run() async {
      var joined = false;
      final diagnostics = Diagnostics(_platform, 'send', sending: true);
      try {
        await previous;
        final ticket = to.id == 'usb' ? null : to.ticket ?? await _platform.readTicket(to.id);
        final wifi = ticket == null ? null : rust.readTicket(text: ticket).wifi;
        await _platform.startTransferService('Connected to ${to.name}');
        _platform.log('send', 'connecting to the other phone (${to.link})');
        await diagnostics.network();
        // Phone to phone is the fastest route, so even on a shared Wi-Fi the sender
        // joins the receiver's own Wi-Fi Direct network first (asking for the
        // permission it needs only now) and connects through it alone. If that
        // can't be done, every address in the code is tried, the router's included.
        String? direct;
        if (wifi != null) {
          if (!(await _platform.readiness()).permission) await _platform.fix(Requirement.permissions);
          direct = await _platform.joinWifi(
            WifiGroup(ssid: wifi.ssid, passphrase: wifi.passphrase, frequency: wifi.frequency),
          );
          joined = direct != null;
          if (!joined) _platform.log('send', 'falling back to the shared network');
        }
        if (cancelled) return;
        final events = ticket == null ? rust.connectUsb() : rust.connect(ticket: ticket, directAddress: direct);
        await for (final event in events) {
          diagnostics.record(event);
          live.apply(event);
          _notify(live.state);
        }
      } on Object catch (e) {
        _platform.log('send', 'connection failed: $e');
        live.emit(live.state.copyWith(phase: LinkPhase.failed, error: friendlyPlatform(e)));
      } finally {
        diagnostics.close();
        await _platform.stopTransferService();
        if (joined) await _platform.leaveWifi();
        if (_sending == live) _sending = null;
        await out.close();
        closed.complete();
      }
    }

    out
      ..onListen = run
      // Leaving the connection page ends the connection, even while it's still
      // joining Wi-Fi Direct.
      ..onCancel = () {
        cancelled = true;
        rust.disconnect();
      };
    return out.stream;
  }

  @override
  void queue(Selection what) {
    final live = _sending;
    if (live == null) return;
    // A very large selection is cut into several batches; the engine says how.
    final sizes = rust.queueFiles(
      sourceToken: what.token,
      files: [for (final f in what.files) rust.SourceFile(path: f.path, size: f.size, modifiedMs: f.modifiedMs)],
    );
    _platform.log('send', 'queued ${what.files.length} files, ${what.bytes} bytes in ${sizes.length} batch(es)');
    var start = 0;
    final added = <Batch>[];
    for (final count in sizes) {
      final part = what.files.sublist(start, start + count);
      added.add(Batch(id: live.queued++, files: count, bytes: part.fold(0, (sum, f) => sum + f.size)));
      start += count;
    }
    // Android's access to these files is given back when their last batch is over.
    live.hold(what.token, [for (final b in added) b.id]);
    live.emit(live.state.copyWith(batches: [...live.state.batches, ...added]));
  }

  @override
  Stream<Connection> receive() {
    final out = StreamController<Connection>();
    final previous = _receiveClosed;
    final closed = Completer<void>();
    _receiveClosed = closed.future;
    var cancelled = false;

    Future<void> run() async {
      final live = _Live(const Connection(), onChange: out.add, receiving: true);
      final diagnostics = Diagnostics(_platform, 'receive', sending: false);
      var serving = false;
      try {
        await previous;
        await _platform.watchUsb();
        if (cancelled) return;
        final events = rust.receive(destDir: _device.downloadDir, journalDir: _device.journalDir);
        await for (final event in events) {
          diagnostics.record(event);
          if (event.kind == rust.EventKind.ready) {
            var tickets = rust.receiveTickets();
            live.emit(live.state.copyWith(ticket: tickets.privateTicket));
            final setup = await _platform.startReceiving(tickets.tapTicket, tickets.publicTicket, tickets.port);
            final group = setup.group;
            // What won't work is said on the page, with what still does.
            live.emit(
              live.state.copyWith(
                problems: [if (group == null) SetupProblem.otherNetworks, if (!setup.listed) SetupProblem.notListed],
              ),
            );
            if (group != null && !cancelled) {
              final wifi = rust.WifiInfo(ssid: group.ssid, passphrase: group.passphrase, frequency: group.frequency);
              tickets = rust.receiveTickets(wifi: wifi);
              await _platform.updateTickets(tickets.tapTicket, tickets.publicTicket, tickets.port);
              live.emit(live.state.copyWith(ticket: tickets.privateTicket));
            }
            unawaited(diagnostics.network());
            continue;
          }
          live.apply(event);
          if (event.kind == rust.EventKind.connected) {
            // Connected: the radios that found this phone have done their job.
            await _platform.stopAdvertising(event.route);
            if (!serving) await _platform.startTransferService('Connected to ${live.state.peer}');
            serving = true;
          }
          _notify(live.state);
        }
      } on Object catch (e) {
        if (!cancelled) {
          _platform.log('receive', 'stopped: $e');
          live.emit(live.state.copyWith(phase: LinkPhase.failed, error: friendlyPlatform(e)));
        }
      } finally {
        diagnostics.close();
        try {
          rust.stopReceiving();
          await _platform.stopReceiving();
          await _platform.stopTransferService();
        } finally {
          await out.close();
          closed.complete();
        }
      }
    }

    out
      ..onListen = run
      // Leaving Receive ends it at once: stopping the engine's receiver ends its
      // events, even while it's idle waiting for a sender.
      ..onCancel = () {
        cancelled = true;
        rust.stopReceiving();
        return closed.future;
      };
    return out.stream;
  }

  @override
  bool answer(int requestId, {required bool accept}) {
    final delivered = rust.answerRequest(requestId: requestId, accept: accept);
    _platform.log('receive', delivered ? (accept ? 'accepted' : 'declined') : 'answer came too late: they had left');
    return delivered;
  }

  @override
  void disconnect() {
    _platform.log('connection', 'disconnecting');
    rust.disconnect();
    rust.disconnectReceiving();
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
    _platform
        .installUpdate(_update!)
        .then(
          (_) => controller.close(),
          onError: (Object e) {
            controller.addError(e);
            controller.close();
          },
        )
        .whenComplete(forward.cancel);
    return controller.stream;
  }

  DateTime _notified = DateTime(0);

  /// The notification needs no more than one update a second.
  void _notify(Connection c) {
    final p = c.progress;
    final now = DateTime.now();
    if (p == null || now.difference(_notified) < const Duration(seconds: 1)) return;
    _notified = now;
    _platform.updateTransferService(
      '${formatBytes(p.doneBytes)} of ${formatBytes(p.totalBytes)}',
      (p.fraction * 100).round(),
    );
  }
}

/// A connection in progress: turns engine events into snapshots for the UI,
/// with a smoothed speed.
class _Live {
  _Live(this.state, {this.onChange, required this.receiving, this.onRelease});

  Connection state;
  final void Function(Connection)? onChange;

  /// Whether this phone is the receiving one: "no space" means a different phone.
  final bool receiving;

  /// Called with a pick's token once none of its batches is left to send.
  final void Function(int token)? onRelease;
  var _everConnected = false;
  int queued = 0;

  /// Batches still to finish, by the pick they came from.
  final _batchToken = <int, int>{};
  final _openBatches = <int, int>{};

  void hold(int token, Iterable<int> batches) {
    for (final id in batches) {
      _batchToken[id] = token;
      _openBatches[token] = (_openBatches[token] ?? 0) + 1;
    }
  }

  void _finished(int batch) {
    final token = _batchToken.remove(batch);
    if (token == null) return;
    final left = (_openBatches[token] ?? 1) - 1;
    if (left > 0) {
      _openBatches[token] = left;
    } else {
      _openBatches.remove(token);
      onRelease?.call(token);
    }
  }

  void _releaseAll() {
    for (final token in _openBatches.keys) {
      onRelease?.call(token);
    }
    _openBatches.clear();
    _batchToken.clear();
  }

  DateTime? _speedAt;
  int _speedBytes = 0;
  double _speed = 0;

  void emit(Connection next) {
    state = next;
    onChange?.call(next);
  }

  void apply(rust.TransferEvent e) {
    final s = state;
    // Files a batch was made from aren't needed once it's over, whichever way it ended.
    switch (e.kind) {
      case rust.EventKind.completed || rust.EventKind.batchFailed:
        _finished(e.batch);
      case rust.EventKind.disconnected || rust.EventKind.failed:
        _releaseAll();
      default:
        break;
    }
    emit(switch (e.kind) {
      rust.EventKind.ready => s,
      rust.EventKind.request => s.copyWith(
        phase: LinkPhase.waiting,
        peer: e.text,
        request: Request(id: e.requestId, from: e.text ?? 'A nearby phone', code: e.code),
      ),
      // The phone that was asking left; the question no longer applies.
      rust.EventKind.requestWithdrawn =>
        s.request?.id == e.requestId ? s.copyWith(phase: LinkPhase.connecting, peer: '', keepRequest: false) : s,
      rust.EventKind.note => s,
      rust.EventKind.code => s.copyWith(phase: LinkPhase.waiting, code: e.code),
      rust.EventKind.connected => _connected(s, e),
      rust.EventKind.started => _start(s, e),
      rust.EventKind.progress => s.moving == null ? s : s.copyWith(progress: _progress(e)),
      rust.EventKind.completed => s.copyWith(batches: _mark(s.batches, e.batch, BatchState.done), keepProgress: false),
      rust.EventKind.batchFailed => s.copyWith(
        batches: _mark(
          s.batches,
          e.batch,
          BatchState.failed,
          error: friendlyFailure(e.failure, peer: s.peer, receiving: receiving),
        ),
        keepProgress: false,
      ),
      // Only a connection that was set up has anything to reconnect.
      rust.EventKind.connectionLost => _everConnected ? s.copyWith(phase: LinkPhase.reconnecting) : s,
      rust.EventKind.peerLeft => s.copyWith(endedByPeer: true),
      rust.EventKind.disconnected => s.copyWith(phase: LinkPhase.ended, keepProgress: false),
      rust.EventKind.failed => s.copyWith(
        phase: LinkPhase.failed,
        error: friendlyFailure(e.failure, peer: s.peer, receiving: receiving),
        keepProgress: false,
      ),
    });
  }

  Connection _connected(Connection s, rust.TransferEvent e) {
    _everConnected = true;
    // The sender keeps the code on screen: it has to be compared before anything is sent.
    return s.copyWith(phase: LinkPhase.connected, peer: e.text, keepRequest: false);
  }

  /// The sender already listed the batch when it was queued; the receiver lists
  /// it when it starts arriving.
  Connection _start(Connection s, rust.TransferEvent e) {
    _speedAt = null;
    _speed = 0;
    final batch = Batch(id: e.batch, files: e.totalFiles, bytes: e.totalBytes, state: BatchState.moving);
    final known = s.batches.any((b) => b.id == e.batch);
    return s.copyWith(
      batches: known ? [for (final b in s.batches) b.id == e.batch ? batch : b] : [...s.batches, batch],
      progress: Progress(totalBytes: e.totalBytes, totalFiles: e.totalFiles),
    );
  }

  List<Batch> _mark(List<Batch> batches, int id, BatchState state, {String? error}) => [
    for (final b in batches) b.id == id ? b.copyWith(state: state, error: error) : b,
  ];

  Progress _progress(rust.TransferEvent e) {
    final now = DateTime.now();
    final at = _speedAt;
    if (at != null && now.difference(at) < const Duration(milliseconds: 500)) return _last(e);
    if (at != null) {
      final instant = (e.doneBytes - _speedBytes) / (now.difference(at).inMicroseconds / 1e6);
      _speed = _speed == 0 ? instant : _speed * 0.7 + instant * 0.3;
    }
    _speedAt = now;
    _speedBytes = e.doneBytes;
    return _last(e);
  }

  Progress _last(rust.TransferEvent e) {
    return Progress(
      doneBytes: e.doneBytes,
      totalBytes: e.totalBytes,
      doneFiles: e.doneFiles,
      totalFiles: e.totalFiles,
      bytesPerSecond: _speed.clamp(0, double.infinity),
    );
  }
}
