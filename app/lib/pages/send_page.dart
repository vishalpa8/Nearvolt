import 'dart:async';

import 'package:flutter/services.dart' show PlatformException;
import 'package:material_ui/material_ui.dart';

import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme.dart';
import '../ui/molecules.dart';
import '../ui/organisms.dart';
import '../ui/templates.dart';
import 'connection_page.dart';

/// The ways to reach a phone with Receive open. Only the one chosen runs, and
/// it asks only for what it needs.
class SendPage extends StatelessWidget {
  const SendPage({super.key, required this.engine, required this.profile});

  final TransferEngine engine;
  final DeviceProfile profile;

  void _open(BuildContext context, Widget page) =>
      Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => page));

  Future<void> _scan(BuildContext context) async {
    void tell(String message) {
      if (context.mounted) ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(message)));
    }

    if ((await engine.needs(Purpose.scan)).contains(Need.wifi)) await engine.fix(Need.wifi);
    final String? ticket;
    try {
      ticket = await engine.scanCode();
    } on PlatformException catch (e) {
      return tell(e.message ?? "Couldn't open the code scanner.");
    }
    if (ticket == null) return;
    final NearbyDevice device;
    try {
      device = engine.deviceFor(ticket);
    } on Object {
      return tell("That isn't a Nearvolt code.");
    }
    if (context.mounted) await openConnection(context, engine, device);
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return PageScaffold(
      title: 'Send',
      onBack: () => Navigator.of(context).maybePop(),
      body: ListView(
        padding: const EdgeInsets.only(top: Space.l, bottom: Space.xl),
        children: [
          Text('How do you want to connect?', style: text.titleLarge),
          const SizedBox(height: Space.xs),
          Text(
            'Open Receive on the other phone first.',
            style: text.bodyMedium?.copyWith(color: AppColors.textSecondary),
          ),
          const SizedBox(height: Space.xl),
          if (profile.hasNfc)
            ChoiceTile(
              icon: Icons.nfc_rounded,
              title: 'Tap phones',
              detail: 'Hold the backs together. Connects at once, nothing to approve.',
              onTap: () => _open(context, _TapPage(engine: engine)),
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
            detail: 'Pick it from a list. The other phone approves with a code.',
            onTap: () => _open(context, _NearbyPage(engine: engine)),
          ),
          ChoiceTile(
            icon: Icons.usb_rounded,
            title: 'USB cable',
            detail: 'Plug the phones into each other. No Wi-Fi needed.',
            onTap: () => _open(context, _UsbPage(engine: engine)),
          ),
        ],
      ),
    );
  }
}

/// Connects to [device] and shows the connection until the user leaves it.
Future<void> openConnection(BuildContext context, TransferEngine engine, NearbyDevice device) {
  final updates = engine.connect(device);
  return Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ConnectionPage(
        engine: engine,
        updates: updates,
        initial: Connection(peer: device.name, link: device.link),
        sending: true,
      ),
    ),
  );
}

/// Shared by the method pages: listens for receivers while the page is on top,
/// and stops listening while a connection it opened is showing.
mixin _MethodMixin<T extends StatefulWidget, F> on State<T>, NeedsMixin<T> {
  StreamSubscription<F>? _sub;

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

  Future<void> connectTo(NearbyDevice device) async {
    _sub?.cancel();
    _sub = null;
    await openConnection(context, engine, device);
    if (mounted) restart();
  }
}

class _TapPage extends StatefulWidget {
  const _TapPage({required this.engine});

  final TransferEngine engine;

  @override
  State<_TapPage> createState() => _TapPageState();
}

class _TapPageState extends State<_TapPage> with NeedsMixin, _MethodMixin<_TapPage, String> {
  @override
  TransferEngine get engine => widget.engine;

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
  const _UsbPage({required this.engine});

  final TransferEngine engine;

  @override
  State<_UsbPage> createState() => _UsbPageState();
}

class _UsbPageState extends State<_UsbPage> with NeedsMixin, _MethodMixin<_UsbPage, NearbyDevice?> {
  @override
  TransferEngine get engine => widget.engine;

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
  const _NearbyPage({required this.engine});

  final TransferEngine engine;

  @override
  State<_NearbyPage> createState() => _NearbyPageState();
}

class _NearbyPageState extends State<_NearbyPage> with NeedsMixin, _MethodMixin<_NearbyPage, List<NearbyDevice>> {
  List<NearbyDevice> _devices = const [];

  @override
  TransferEngine get engine => widget.engine;

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
          const SizedBox(height: Space.m),
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
