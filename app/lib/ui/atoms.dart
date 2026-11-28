import 'dart:math';

import 'package:material_ui/material_ui.dart';
import 'package:qr/qr.dart';

import '../engine/models.dart';
import '../theme.dart';

class Dot extends StatelessWidget {
  const Dot({super.key, required this.color, this.size = 8});

  final Color color;
  final double size;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(color: color, shape: BoxShape.circle),
    );
  }
}

enum PillStyle { tinted, outlined, tag }

/// Small rounded label: a status with a colored dot, an outlined fact, or a tag.
class Pill extends StatelessWidget {
  const Pill({super.key, required this.label, this.color = AppColors.accent, this.style = PillStyle.tinted});

  final String label;
  final Color color;
  final PillStyle style;

  @override
  Widget build(BuildContext context) {
    final outlined = style == PillStyle.outlined;
    // The words are darker than the dot and fill: pale green or amber text on its own tint is hard to read.
    final fill = Color.alphaBlend(color.withValues(alpha: 0.1), AppColors.background);
    final textColor = outlined ? AppColors.textPrimary : readableOn(color, fill);
    return Container(
      padding: EdgeInsets.symmetric(horizontal: Space.m, vertical: outlined ? Space.s : 6),
      decoration: BoxDecoration(
        color: switch (style) {
          PillStyle.tinted => color.withValues(alpha: 0.1),
          PillStyle.outlined => AppColors.surface,
          PillStyle.tag => AppColors.accentSoft,
        },
        border: outlined ? Border.all(color: AppColors.border) : null,
        borderRadius: BorderRadius.circular(100),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (style != PillStyle.tag) ...[Dot(color: color), const SizedBox(width: Space.s)],
          Flexible(
            child: Text(
              label,
              overflow: TextOverflow.ellipsis,
              style: Theme.of(context).textTheme.labelLarge?.copyWith(color: textColor),
            ),
          ),
        ],
      ),
    );
  }
}

class DeviceIcon extends StatelessWidget {
  const DeviceIcon({super.key, required this.kind, this.size = 64, this.selected = false});

  final DeviceKind kind;
  final double size;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final icon = switch (kind) {
      DeviceKind.phone => Icons.smartphone_rounded,
      DeviceKind.tablet => Icons.tablet_mac_rounded,
      DeviceKind.laptop => Icons.laptop_mac_rounded,
    };
    return AnimatedContainer(
      duration: const Duration(milliseconds: 200),
      curve: Curves.easeOut,
      width: size,
      height: size,
      decoration: BoxDecoration(shape: BoxShape.circle, color: selected ? AppColors.accent : AppColors.accentSoft),
      child: Icon(icon, size: size * 0.42, color: selected ? Colors.white : AppColors.accent),
    );
  }
}

class ProgressRing extends StatelessWidget {
  const ProgressRing({
    super.key,
    required this.fraction,
    required this.color,
    required this.child,
    this.size = 248,
    this.stroke = 14,
  });

  final double fraction;
  final Color color;
  final Widget child;
  final double size;
  final double stroke;

  @override
  Widget build(BuildContext context) {
    return TweenAnimationBuilder<double>(
      tween: Tween(end: fraction.clamp(0.0, 1.0)),
      duration: const Duration(milliseconds: 300),
      curve: Curves.easeOut,
      builder: (context, value, child) => CustomPaint(
        painter: _RingPainter(value, color, stroke),
        child: SizedBox.square(
          dimension: size,
          child: Center(child: child),
        ),
      ),
      child: child,
    );
  }
}

class _RingPainter extends CustomPainter {
  _RingPainter(this.value, this.color, this.stroke);

  final double value;
  final Color color;
  final double stroke;

  @override
  void paint(Canvas canvas, Size size) {
    final arc = (Offset.zero & size).deflate(stroke / 2);
    final paint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = stroke
      ..strokeCap = StrokeCap.round;
    canvas.drawOval(arc, paint..color = AppColors.surfaceMuted);
    if (value > 0) {
      canvas.drawArc(arc, -pi / 2, 2 * pi * value, false, paint..color = color);
    }
  }

  @override
  bool shouldRepaint(_RingPainter old) => old.value != value || old.color != color || old.stroke != stroke;
}

/// A QR code with its quiet zone, drawn as crisp squares at any size.
class QrView extends StatelessWidget {
  const QrView({super.key, required this.data, this.size = 232});

  final String data;
  final double size;

  @override
  Widget build(BuildContext context) {
    final image = QrImage(QrCode(payload: QrPayload.fromString(data)));
    return Semantics(
      label: 'QR code to connect',
      child: CustomPaint(size: Size.square(size), painter: _QrPainter(image)),
    );
  }
}

class _QrPainter extends CustomPainter {
  _QrPainter(this.image);

  final QrImage image;

  @override
  void paint(Canvas canvas, Size size) {
    const quiet = 4;
    final n = image.moduleCount;
    final cell = size.width / (n + 2 * quiet);
    final paint = Paint()..color = AppColors.textPrimary;
    for (var row = 0; row < n; row++) {
      var col = 0;
      while (col < n) {
        if (!image.isDark(row, col)) {
          col++;
          continue;
        }
        final start = col;
        while (col < n && image.isDark(row, col)) {
          col++;
        }
        canvas.drawRect(Rect.fromLTWH((start + quiet) * cell, (row + quiet) * cell, (col - start) * cell, cell), paint);
      }
    }
  }

  @override
  bool shouldRepaint(_QrPainter old) => old.image != image;
}

/// Bordered white surface used for cards across the app.
class Panel extends StatelessWidget {
  const Panel({super.key, required this.child, this.padding = const EdgeInsets.all(Space.xl)});

  final Widget child;
  final EdgeInsetsGeometry padding;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: padding,
      decoration: BoxDecoration(
        color: AppColors.surface,
        border: Border.all(color: AppColors.border),
        borderRadius: BorderRadius.circular(Radii.card),
      ),
      child: child,
    );
  }
}
