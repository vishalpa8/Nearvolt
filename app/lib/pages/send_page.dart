import 'dart:async';

import 'package:flutter/services.dart' show PlatformException;
import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/messages.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../debug.dart';
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

  /// The picked files that no connection has taken yet. Once one took them (sent,
  /// cancelled or failed) the pick is used up and its files are released, so it is not
  /// shown or offered again: offering it again made the next transfer fail to read them.
  Selection? get _unsent {
    final picked = _files;
    return picked == null || engine.taken(picked) ? null : picked;
  }

  TransferEngine get engine => widget.engine;

  /// A second tap while a screen or the scanner is opening does nothing, so one tap
  /// can't start two connections.
  var _busy = false;

  Future<void> _once(Future<void> Function() action) async {
    if (_busy) {
      DebugLog.log('tap', 'ignored: the previous action has not finished');
      return;
    }
    _busy = true;
    try {
      await action();
    } finally {
      _busy = false;
      // Back on this screen: files that were just sent (or cancelled) are no longer selected.
      if (mounted && _files != null && engine.taken(_files!)) setState(() => _files = null);
    }
  }

  Future<void> _open(BuildContext context, Widget page) => _once(
    () => Navigator.of(context).push(
      MaterialPageRoute<void>(
        settings: RouteSettings(name: page.runtimeType.toString()),
        builder: (_) => page,
      ),
    ),
  );

  Future<void> _choose() async {
    final kind = await showPickMenu(context);
    if (kind == null || !mounted) return;
    final picked = await pickSelection(context, engine, kind);
    if (picked == null || !mounted) return;
    if (picked.skipped > 0) {
      showNote(context, leftOut(picked.skipped));
    }
    if (picked.files.isNotEmpty) setState(() => _files = picked);
  }

  Future<void> _scan(BuildContext context) => _once(() => _scanOnce(context));

  Future<void> _scanOnce(BuildContext context) async {
    void tell(String message) {
      if (context.mounted) showNote(context, message);
    }

    // The camera stays closed until what a transfer needs is on.
    if (!await ensureReady(context, engine, Purpose.scan)) return;
    final String? ticket;
    try {
      // A scanner that never answers must not leave the buttons locked for ever.
      ticket = await engine.scanCode().timeout(const Duration(minutes: 3), onTimeout: () => null);
    } on PlatformException catch (e) {
      return tell(friendlyPlatform(e));
    }
    if (ticket == null) return;
    final Peer device;
    try {
      device = engine.deviceFor(ticket);
    } on Object {
      return tell("That isn't a Nearvolt code.");
    }
    if (context.mounted) await openConnection(context, engine, device, files: _unsent);
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final files = _unsent;
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
              onTap: () => _open(context, _TapPage(engine: engine, files: _unsent)),
            ),
          ChoiceTile(
            icon: Icons.qr_code_scanner_rounded,
            title: 'Scan code',
            detail: "Point the camera at the code on the other phone's screen. Connects at once.",
            onTap: () => _scan(context),
          ),
          ChoiceTile(
            icon: Icons.usb_rounded,
            title: 'USB cable',
            detail: 'Plug the phones into each other. No Wi-Fi needed.',
            onTap: () => _open(context, _UsbPage(engine: engine, files: _unsent)),
          ),
        ],
      ),
    );
  }
}

/// Connects to [device] and shows the connection until the user leaves it.
/// [files], if chosen first, are sent once the connection is set up.
Future<void> openConnection(BuildContext context, TransferEngine engine, Peer device, {Selection? files}) {
  final updates = engine.connect(device);
  return Navigator.of(context).push(
    MaterialPageRoute<void>(
      settings: const RouteSettings(name: 'ConnectionPage (sending)'),
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
  void pause() {
    _sub?.cancel();
    _sub = null;
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  var _connecting = false;

  /// Opens the connection. A second tap, or a second tap-read, while one is opening
  /// does nothing, so there's never two connections to the same phone.
  Future<void> connectTo(Peer device) async {
    if (_connecting) return;
    _connecting = true;
    _sub?.cancel();
    _sub = null;
    try {
      // Files picked before and already sent (or cancelled) are not sent again.
      final picked = files;
      await openConnection(context, engine, device, files: picked == null || engine.taken(picked) ? null : picked);
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
    final Peer device;
    try {
      device = engine.deviceFor(ticket);
    } on Object {
      showNote(context, "That phone didn't share a Nearvolt code.");
      return;
    }
    connectTo(device);
  }

  @override
  Widget build(BuildContext context) =>
      blockedPage('Tap phones') ??
      _Waiting(
        title: 'Tap phones',
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

class _UsbPageState extends State<_UsbPage> with NeedsMixin, _MethodMixin<_UsbPage, Peer?> {
  @override
  TransferEngine get engine => widget.engine;

  @override
  Selection? get files => widget.files;

  @override
  Purpose get purpose => Purpose.usb;

  @override
  Stream<Peer?> find() => engine.usbPeer();

  @override
  void found(Peer? device) {
    if (device != null) connectTo(device);
  }

  @override
  Widget build(BuildContext context) =>
      blockedPage('USB cable') ??
      _Waiting(
        title: 'USB cable',
        icon: Icons.usb_rounded,
        headline: 'Connect the phones with a USB cable',
        detail: 'A USB-C to USB-C cable, or an adapter. The other phone needs Receive open and asks once to allow it.',
      );
}

/// A method page with nothing to choose: one picture and what to do.
class _Waiting extends StatelessWidget {
  const _Waiting({required this.title, required this.icon, required this.headline, required this.detail});

  final String title;
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
          Expanded(
            child: LayoutBuilder(
              builder: (context, box) => SingleChildScrollView(
                child: ConstrainedBox(
                  constraints: BoxConstraints(minHeight: box.maxHeight),
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
              ),
            ),
          ),
        ],
      ),
    );
  }
}
