import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../engine/demo_engine.dart';
import '../engine/engine.dart';
import '../engine/models.dart';
import '../theme/tokens.dart';
import '../widgets/device_avatar.dart';
import 'pick_screen.dart';
import 'receive_sheet.dart';
import 'transfer_screen.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key, required this.engine, this.showSampleOffer = false});

  final TransferEngine engine;

  /// Opens the receive prompt right away, for design review.
  final bool showSampleOffer;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  List<NearbyDevice> _devices = const [];
  late final StreamSubscription<List<NearbyDevice>> _deviceSub;
  late final StreamSubscription<IncomingOffer> _offerSub;

  @override
  void initState() {
    super.initState();
    _deviceSub = widget.engine.nearbyDevices().listen((devices) => setState(() => _devices = devices));
    _offerSub = widget.engine.incomingOffers().listen(_onOffer);
    if (widget.showSampleOffer) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _onOffer(DemoEngine.sampleOffer));
    }
  }

  @override
  void dispose() {
    _deviceSub.cancel();
    _offerSub.cancel();
    super.dispose();
  }

  Future<void> _choose(NearbyDevice? device) async {
    final items = await widget.engine.pickFiles();
    if (items.isEmpty || !mounted) return;
    await Navigator.of(context).push(MaterialPageRoute<void>(
      builder: (_) => PickScreen(engine: widget.engine, items: items, devices: _devices, initial: device),
    ));
  }

  Future<void> _onOffer(IncomingOffer offer) async {
    final accepted = await showReceiveSheet(context, offer);
    if (!mounted) return;
    if (!accepted) {
      widget.engine.decline(offer);
      return;
    }
    await Navigator.of(context).push(MaterialPageRoute<void>(
      builder: (_) => TransferScreen(progress: widget.engine.receive(offer), peer: offer.from, incoming: true),
    ));
  }

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final engine = widget.engine;
    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Space.xl),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const SizedBox(height: Space.l),
              Row(
                children: [
                  Text('Proxima', style: text.titleLarge),
                  const Spacer(),
                  if (engine is DemoEngine) _PreviewBadge(onLongPress: engine.simulateIncoming),
                ],
              ),
              const SizedBox(height: Space.xxxl),
              Text('Send to anyone nearby', style: text.displaySmall),
              const SizedBox(height: Space.s),
              Text(
                'No internet or account needed. Every file arrives exactly as it was sent.',
                style: text.bodyLarge?.copyWith(color: AppColors.textSecondary),
              ),
              const SizedBox(height: Space.xl),
              _VisibleAs(name: engine.deviceName),
              const SizedBox(height: Space.xxl),
              Text('Nearby', style: text.titleMedium),
              const SizedBox(height: Space.m),
              Expanded(
                child: SingleChildScrollView(
                  child: AnimatedSwitcher(
                    duration: const Duration(milliseconds: 250),
                    child: _devices.isEmpty
                        ? const _Searching()
                        : Wrap(
                            key: const ValueKey('devices'),
                            spacing: Space.s,
                            runSpacing: Space.l,
                            children: [
                              for (final device in _devices)
                                DeviceAvatar(device: device, onTap: () => _choose(device)),
                            ],
                          ),
                  ),
                ),
              ),
              FilledButton.icon(
                onPressed: () => _choose(null),
                icon: const Icon(Icons.add_rounded),
                label: const Text('Choose files'),
              ),
              const SizedBox(height: Space.xl),
            ],
          ),
        ),
      ),
    );
  }
}

class _VisibleAs extends StatelessWidget {
  const _VisibleAs({required this.name});

  final String name;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: Space.m, vertical: Space.s),
      decoration: BoxDecoration(
        color: AppColors.surface,
        border: Border.all(color: AppColors.border),
        borderRadius: BorderRadius.circular(100),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: 8,
            height: 8,
            decoration: const BoxDecoration(color: AppColors.success, shape: BoxShape.circle),
          ),
          const SizedBox(width: Space.s),
          Text('Visible as $name', style: Theme.of(context).textTheme.labelLarge),
        ],
      ),
    );
  }
}

class _Searching extends StatelessWidget {
  const _Searching();

  @override
  Widget build(BuildContext context) {
    return Padding(
      key: const ValueKey('searching'),
      padding: const EdgeInsets.symmetric(vertical: Space.l),
      child: Row(
        children: [
          const SizedBox.square(
            dimension: 18,
            child: CircularProgressIndicator(strokeWidth: 2, color: AppColors.accent),
          ),
          const SizedBox(width: Space.m),
          Text(
            'Looking for nearby devices…',
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(color: AppColors.textSecondary),
          ),
        ],
      ),
    );
  }
}

/// Makes it obvious that devices and transfers are simulated. Long-press to
/// preview an incoming transfer.
class _PreviewBadge extends StatelessWidget {
  const _PreviewBadge({required this.onLongPress});

  final VoidCallback onLongPress;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: 'Devices and transfers are simulated. Long-press to preview receiving.',
      child: GestureDetector(
        onLongPress: onLongPress,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: Space.m, vertical: 6),
          decoration: BoxDecoration(color: AppColors.accentSoft, borderRadius: BorderRadius.circular(100)),
          child: Text(
            'Preview',
            style: Theme.of(context).textTheme.labelLarge?.copyWith(color: AppColors.accent),
          ),
        ),
      ),
    );
  }
}
