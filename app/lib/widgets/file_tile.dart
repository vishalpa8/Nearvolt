import 'package:material_ui/material_ui.dart';

import '../engine/models.dart';
import '../theme/tokens.dart';
import '../util/format.dart';

class FileTile extends StatelessWidget {
  const FileTile({super.key, required this.item, this.onRemove});

  final PickedItem item;
  final VoidCallback? onRemove;

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
          if (onRemove != null)
            IconButton(
              onPressed: onRemove,
              tooltip: 'Remove ${item.name}',
              icon: const Icon(Icons.close_rounded, color: AppColors.textSecondary),
            ),
        ],
      ),
    );
  }
}
