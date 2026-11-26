import 'package:material_ui/material_ui.dart';

import '../engine/models.dart';
import '../theme.dart';
import 'atoms.dart';
import 'format.dart';

/// A nearby device: icon, name and how it connects.
class DeviceTile extends StatelessWidget {
  const DeviceTile({super.key, required this.device, this.selected = false, this.onTap});

  final NearbyDevice device;
  final bool selected;
  final VoidCallback? onTap;

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
                DeviceIcon(kind: device.kind, selected: selected),
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

class FileRow extends StatelessWidget {
  const FileRow({super.key, required this.item});

  final PickedItem item;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final detail = item.isFolder ? '${filesLabel(item.files)} · ${formatBytes(item.bytes)}' : formatBytes(item.bytes);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Space.s),
      child: Row(
        children: [
          Container(
            width: 44,
            height: 44,
            decoration: BoxDecoration(
              color: AppColors.surfaceMuted,
              borderRadius: BorderRadius.circular(Radii.button),
            ),
            child: Icon(
              item.isFolder ? Icons.folder_rounded : Icons.insert_drive_file_rounded,
              color: AppColors.textSecondary,
              size: 22,
            ),
          ),
          const SizedBox(width: Space.m),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(item.name, style: text.bodyLarge, maxLines: 1, overflow: TextOverflow.ellipsis),
                Text(detail, style: text.bodySmall),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class StatCell extends StatelessWidget {
  const StatCell({super.key, required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Expanded(
      child: Column(
        children: [
          Text(label, style: text.bodySmall),
          const SizedBox(height: Space.xs),
          Text(
            value,
            style: text.titleMedium?.copyWith(fontFeatures: tabularFigures),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
        ],
      ),
    );
  }
}

/// Total size and file count of a selection.
class SummaryCard extends StatelessWidget {
  const SummaryCard({super.key, required this.files, required this.bytes});

  final int files;
  final int bytes;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Panel(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(formatBytes(bytes), style: text.displaySmall?.copyWith(fontFeatures: tabularFigures)),
          const SizedBox(height: Space.xs),
          Text(filesLabel(files), style: text.bodyLarge?.copyWith(color: AppColors.textSecondary)),
        ],
      ),
    );
  }
}
