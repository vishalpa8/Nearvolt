import 'dart:async';

import 'package:flutter/services.dart' show PlatformException;
import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/messages.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/atoms.dart';
import '../ui/format.dart';
import '../ui/molecules.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'connection_page.dart';

/// The ways to reach a phone with Receive open. Only the one chosen runs, and
/// it asks only for what it needs. Files can be chosen first, and then go as
/// soon as the connection is set up, or after connecting.
class SendPage extends StatefulWidget {
  const SendPage({super.key, required this.engine, required this.profile});

  final TransferEngine engine;
  final DeviceProfile profile;

  @override
  State<SendPage> createState() => _SendPageState();
}

class _SendPageState extends State<SendPage> {
  Selection? _files;

  TransferEngine get engine => widget.engine;

  /// A second tap while a screen or the scanner is opening does nothing, so one tap
  /// can't start two connections.
  var _busy = false;

  Future<void> _once(Future<void> Function() action) async {
    if (_busy) return;
    _busy = true;
    try {
      await action();
    } finally {
      _busy = false;
    }
  }

  Future<void> _open(BuildContext context, Widget page) =>
      _once(() => Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => page)));

  Future<void> _choose() async {
    final kind = await showPickMenu(context);
    if (kind == null || !mounted) return;
    final picked = await pickSelection(context, engine, kind);
    if (picked == null || !mounted) return;
    if (picked.skipped > 0) {
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(leftOut(picked.skipped))));
    }
    if (picked.files.isNotEmpty) setState(() => _files = picked);
  }

  Future<void> _scan(BuildContext context) => _once(() => _scanOnce(context));

  Future<void> _scanOnce(BuildContext context) async {
    void tell(String message) {
      if (context.mounted) ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(message)));
    }

    if ((await engine.needs(Purpose.scan)).contains(Need.wifi)) await engine.fix(Need.wifi);
    final String? ticket;
    try {
      ticket = await engine.scanCode();
    } on PlatformException catch (e) {
      return tell(friendlyPlatform(e));
    }
    if (ticket == null) return;
    final NearbyDevice device;
    try {
      device = engine.deviceFor(ticket);
    } on Object {
      return tell("That isn't a Nearvolt code.");
    }
    if (context.mounted) await openConnection(context, engine, device, files: _files);
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final files = _files;
    return PageScaffold(
      title: 'Send',
      onBack: () => Navigator.of(context).maybePop(),
      body: ListView(
        padding: const EdgeInsets.only(top: Space.l, bottom: Space.xl),
        children: [
          Text('What do you want to send?', style: text.titleLarge),
          const SizedBox(height: Space.xs),
          Text(
            files == null
                ? 'Choose now and it starts as soon as you connect, or connect first and add files after.'
                : 'Sent as soon as the connection is set up.',
            style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
          ),
          const SizedBox(height: Space.l),
          if (files == null)
            ChoiceTile(
              icon: Icons.add_rounded,
              title: 'Choose files',
              detail: 'Optional. Files or a folder.',
              onTap: _choose,
            )
          else
            Panel(
              child: Row(
                children: [
                  const Icon(Icons.insert_drive_file_outlined, color: AppColors.accent),
                  const SizedBox(width: Space.m),
                  Expanded(
                    child: Text(
                      '${filesLabel(files.files.length)} · ${formatBytes(files.bytes)}',
                      style: text.bodyLarge,
                    ),
                  ),
                  TextButton(onPressed: _choose, child: const Text('Change')),
                  IconButton(
                    onPressed: () => setState(() => _files = null),
                    tooltip: 'Remove',
                    icon: const Icon(Icons.close_rounded, color: AppColors.textSecondary),
                  ),
                ],
              ),
            ),
          const SizedBox(height: Space.xl),
          Text('How do you want to connect?', style: text.titleMedium),
          const SizedBox(height: Space.xs),
          Text(
            'Open Receive on the other phone first.',
            style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
          ),
          const SizedBox(height: Space.l),
          if (widget.profile.hasNfc)
            ChoiceTile(
              icon: Icons.nfc_rounded,
              title: 'Tap phones',
              detail: 'Hold the backs together. Connects at once, nothing to approve.',
              onTap: () => _open(context, _TapPage(engine: engine, files: _files)),
            ),
          ChoiceTile(
            icon: Icons.qr_code_scanner_rounded,
            title: 'Scan code',
            detail: "Point the camera at the code on the other phone's screen.",
            onTap: () => _scan(context),
          ),
          ChoiceTile(
            icon: Icons.radar_rounded,
            title: 'Nearby phones',
            detail: 'Pick it from a list when you are on the same Wi-Fi. The other phone approves with a code.',
            onTap: () => _open(context, _NearbyPage(engine: engine, files: _files)),
          ),
          ChoiceTile(
            icon: Icons.usb_rounded,
            title: 'USB cable',
            detail: 'Plug the phones into each other. No Wi-Fi needed.',
            onTap: () => _open(context, _UsbPage(engine: engine, files: _files)),
          ),
        ],
      ),
    );
  }
}

/// Connects to [device] and shows the connection until the user leaves it.
/// [files], if chosen first, are sent once the connection is set up.
Future<void> openConnection(BuildContext context, TransferEngine engine, NearbyDevice device, {Selection? files}) {
  final updates = engine.connect(device);
  return Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ConnectionPage(
        engine: engine,
        updates: updates,
        initial: Connection(peer: device.name, link: device.link),
        sending: true,
        files: files,
      ),
    ),
  );
}

/// Shared by the method pages: listens for receivers while the page is on top,
/// and stops listening while a connection it opened is showing.
mixin _MethodMixin<T extends StatefulWidget, F> on State<T>, NeedsMixin<T> {
  StreamSubscription<F>? _sub;

  /// Chosen on the Send page before connecting, if at all.
  Selection? get files;

  Stream<F> find();

  void found(F value);

  @override
  void restart() {
    _sub?.cancel();
    _sub = find().listen(found);
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  var _connecting = false;

  /// Opens the connection. A second tap, or a second tap-read, while one is opening
  /// does nothing, so there's never two connections to the same phone.
  Future<void> connectTo(NearbyDevice device) async {
    if (_connecting) return;
    _connecting = true;
    _sub?.cancel();
    _sub = null;
    try {
      await openConnection(context, engine, device, files: files);
    } finally {
      _connecting = false;
    }
    if (mounted) restart();
  }
}

class _TapPage extends StatefulWidget {
  const _TapPage({required this.engine, this.files});

  final TransferEngine engine;
  final Selection? files;

  @override
  State<_TapPage> createState() => _TapPageState();
}

class _TapPageState extends State<_TapPage> with NeedsMixin, _MethodMixin<_TapPage, String> {
  @override
  TransferEngine get engine => widget.engine;

  @override
  Selection? get files => widget.files;

  @override
  Purpose get purpose => Purpose.tap;

  @override
  Stream<String> find() => engine.taps();

  @override
  void found(String ticket) {
    final NearbyDevice device;
    try {
      device = engine.deviceFor(ticket);
    } on Object {
      ScaffoldMessenger.of(context)
          .showSnackBar(const SnackBar(content: Text("That phone didn't share a Nearvolt code.")));
      return;
    }
    connectTo(device);
  }

  @override
  Widget build(BuildContext context) => _Waiting(
    title: 'Tap phones',
    notice: needNotice,
    icon: Icons.nfc_rounded,
    headline: 'Hold the backs of the phones together',
    detail: 'Keep them still for a moment. The other phone needs Receive open.',
  );
}

class _UsbPage extends StatefulWidget {
  const _UsbPage({required this.engine, this.files});

  final TransferEngine engine;
  final Selection? files;

  @override
  State<_UsbPage> createState() => _UsbPageState();
}

class _UsbPageState extends State<_UsbPage> with NeedsMixin, _MethodMixin<_UsbPage, NearbyDevice?> {
  @override
  TransferEngine get engine => widget.engine;

  @override
  Selection? get files => widget.files;

  @override
  Purpose get purpose => Purpose.usb;

  @override
  Stream<NearbyDevice?> find() => engine.usbPeer();

  @override
  void found(NearbyDevice? device) {
    if (device != null) connectTo(device);
  }

  @override
  Widget build(BuildContext context) => _Waiting(
    title: 'USB cable',
    notice: needNotice,
    icon: Icons.usb_rounded,
    headline: 'Connect the phones with a USB cable',
    detail: 'A USB-C to USB-C cable, or an adapter. The other phone needs Receive open and asks once to allow it.',
  );
}

class _NearbyPage extends StatefulWidget {
  const _NearbyPage({required this.engine, this.files});

  final TransferEngine engine;
  final Selection? files;

  @override
  State<_NearbyPage> createState() => _NearbyPageState();
}

class _NearbyPageState extends State<_NearbyPage> with NeedsMixin, _MethodMixin<_NearbyPage, List<NearbyDevice>> {
  List<NearbyDevice> _devices = const [];

  @override
  TransferEngine get engine => widget.engine;

  @override
  Selection? get files => widget.files;

  @override
  Purpose get purpose => Purpose.nearby;

  @override
  Stream<List<NearbyDevice>> find() => engine.nearby();

  @override
  void found(List<NearbyDevice> devices) => setState(() => _devices = devices);

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return PageScaffold(
      title: 'Nearby phones',
      onBack: () => Navigator.of(context).maybePop(),
      body: ListView(
        padding: const EdgeInsets.only(bottom: Space.xl),
        children: [
          if (needNotice case final notice?) ...[const SizedBox(height: Space.l), notice],
          const SizedBox(height: Space.xl),
          Text('Nearby', style: text.titleMedium),
          const SizedBox(height: Space.xs),
          Text(
            'Phones on the same Wi-Fi with Receive open. For phones on different networks, use Scan code or Tap.',
            style: text.bodySmall?.copyWith(color: AppColors.textSecondary),
          ),
          const SizedBox(height: Space.m),
          // Searching the network couldn't start: say so, instead of an empty list that
          // looks like nobody is there.
          ValueListenableBuilder<bool>(
            valueListenable: engine.searchFailed,
            builder: (context, failed, _) => failed
                ? Padding(
                    padding: const EdgeInsets.only(bottom: Space.m),
                    child: Panel(
                      child: Row(
                        children: [
                          const Icon(Icons.info_outline_rounded, color: AppColors.warning),
                          const SizedBox(width: Space.m),
                          Expanded(child: Text(searchFailedText, style: text.bodyMedium)),
                        ],
                      ),
                    ),
                  )
                : const SizedBox.shrink(),
          ),
          DeviceList(devices: _devices, onTap: connectTo),
        ],
      ),
    );
  }
}

/// A method page with nothing to choose: one picture and what to do.
class _Waiting extends StatelessWidget {
  const _Waiting({
    required this.title,
    required this.notice,
    required this.icon,
    required this.headline,
    required this.detail,
  });

  final String title;
  final Widget? notice;
  final IconData icon;
  final String headline;
  final String detail;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return PageScaffold(
      title: title,
      onBack: () => Navigator.of(context).maybePop(),
      body: Column(
        children: [
          if (notice case final notice?) ...[const SizedBox(height: Space.l), notice],
          Expanded(
            child: Center(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Container(
                    width: 120,
                    height: 120,
                    decoration: const BoxDecoration(shape: BoxShape.circle, color: AppColors.accentSoft),
                    child: Icon(icon, size: 56, color: AppColors.accent),
                  ),
                  const SizedBox(height: Space.xl),
                  Text(headline, style: text.titleLarge, textAlign: TextAlign.center),
                  const SizedBox(height: Space.s),
                  Text(
                    detail,
                    style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
                    textAlign: TextAlign.center,
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
