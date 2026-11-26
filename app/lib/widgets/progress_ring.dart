import 'dart:math';

import 'package:material_ui/material_ui.dart';

import '../theme/tokens.dart';

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
        child: SizedBox.square(dimension: size, child: Center(child: child)),
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
    final rect = Offset.zero & size;
    final arc = rect.deflate(stroke / 2);
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
