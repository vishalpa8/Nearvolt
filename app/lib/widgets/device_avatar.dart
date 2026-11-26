import 'package:material_ui/material_ui.dart';

import '../engine/models.dart';
import '../theme/tokens.dart';

class DeviceAvatar extends StatelessWidget {
  const DeviceAvatar({super.key, required this.device, this.selected = false, this.onTap, this.size = 64});

  final NearbyDevice device;
  final bool selected;
  final VoidCallback? onTap;
  final double size;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Semantics(
      button: onTap != null,
      selected: selected,
      label: '${device.name}, ${device.link}',
      excludeSemantics: true,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(Radii.card),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Space.xs, vertical: Space.s),
          child: SizedBox(
            width: 112,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                DeviceIcon(kind: device.kind, size: size, selected: selected),
                const SizedBox(height: Space.s),
                Text(
                  device.name,
                  maxLines: 2,
                  textAlign: TextAlign.center,
                  overflow: TextOverflow.ellipsis,
                  style: text.labelLarge,
                ),
                const SizedBox(height: 2),
                Text(device.link, style: text.bodySmall, textAlign: TextAlign.center),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class DeviceIcon extends StatelessWidget {
  const DeviceIcon({super.key, required this.kind, this.size = 64, this.selected = false});

  final DeviceKind kind;
  final double size;
  final bool selected;

  IconData get _icon => switch (kind) {
        DeviceKind.phone => Icons.smartphone_rounded,
        DeviceKind.tablet => Icons.tablet_mac_rounded,
        DeviceKind.laptop => Icons.laptop_mac_rounded,
      };

  @override
  Widget build(BuildContext context) {
    return AnimatedContainer(
      duration: const Duration(milliseconds: 200),
      curve: Curves.easeOut,
      width: size,
      height: size,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: selected ? AppColors.accent : AppColors.accentSoft,
      ),
      child: Icon(_icon, size: size * 0.42, color: selected ? Colors.white : AppColors.accent),
    );
  }
}
