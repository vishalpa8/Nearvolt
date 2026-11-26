import 'package:material_ui/material_ui.dart';

import '../engine/models.dart';
import '../theme/tokens.dart';
import '../util/format.dart';
import '../widgets/device_avatar.dart';

/// Asks whether to accept an incoming transfer. Resolves to true if accepted.
Future<bool> showReceiveSheet(BuildContext context, IncomingOffer offer) async {
  final accepted = await showModalBottomSheet<bool>(
    context: context,
    builder: (_) => _ReceiveSheet(offer: offer),
  );
  return accepted ?? false;
}

class _ReceiveSheet extends StatelessWidget {
  const _ReceiveSheet({required this.offer});

  final IncomingOffer offer;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(Space.xl, 0, Space.xl, Space.xl),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            DeviceIcon(kind: offer.from.kind, size: 72),
            const SizedBox(height: Space.l),
            Text(
              '${offer.from.name} wants to send you',
              style: text.bodyLarge?.copyWith(color: AppColors.textSecondary),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: Space.xs),
            Text(
              '${filesLabel(offer.files)} · ${formatBytes(offer.bytes)}',
              style: text.titleLarge,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: Space.s),
            Text(
              'Saved to Downloads › Nearvolt. Every file is checked as it arrives.',
              style: text.bodySmall,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: Space.xl),
            Row(
              children: [
                Expanded(
                  child: OutlinedButton(
                    onPressed: () => Navigator.of(context).pop(false),
                    child: const Text('Decline'),
                  ),
                ),
                const SizedBox(width: Space.m),
                Expanded(
                  child: FilledButton(
                    onPressed: () => Navigator.of(context).pop(true),
                    child: const Text('Accept'),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
