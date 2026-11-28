import 'dart:math' as math;

import 'package:material_ui/material_ui.dart';

/// Design tokens. Keep in sync with docs/design.md.
abstract final class AppColors {
  static const background = Color(0xFFF7F7F5);
  static const surface = Color(0xFFFFFFFF);
  static const surfaceMuted = Color(0xFFEFEFEC);
  static const border = Color(0xFFE4E4E0);
  static const textPrimary = Color(0xFF17181A);
  static const textSecondary = Color(0xFF6B6E73);
  static const accent = Color(0xFF2457F5);
  static const accentSoft = Color(0xFFE8EEFE);
  static const success = Color(0xFF1F9D55);
  static const warning = Color(0xFFC27C0E);
  static const danger = Color(0xFFD93636);
}

/// WCAG contrast ratio between two colours: 1 (none) to 21 (black on white).
double contrastRatio(Color a, Color b) {
  final la = a.computeLuminance(), lb = b.computeLuminance();
  return (math.max(la, lb) + 0.05) / (math.min(la, lb) + 0.05);
}

/// [color], made darker only as far as needed to read clearly on [background]
/// (4.5:1, the minimum for small text). The brand greens and ambers are too light to
/// carry words on their own tinted fill.
Color readableOn(Color color, Color background, {double ratio = 4.5}) {
  var hsl = HSLColor.fromColor(color);
  var result = color;
  while (contrastRatio(result, background) < ratio && hsl.lightness > 0.05) {
    hsl = hsl.withLightness(hsl.lightness - 0.02);
    result = hsl.toColor();
  }
  return result;
}

abstract final class Space {
  static const double xs = 4, s = 8, m = 12, l = 16, xl = 24, xxl = 32, xxxl = 48;
}

abstract final class Radii {
  static const double button = 12, card = 20, sheet = 28;
}

const tabularFigures = [FontFeature.tabularFigures()];

ThemeData buildTheme() {
  const scheme = ColorScheme.light(
    primary: AppColors.accent,
    onPrimary: Colors.white,
    primaryContainer: AppColors.accentSoft,
    onPrimaryContainer: AppColors.accent,
    secondary: AppColors.textSecondary,
    surface: AppColors.surface,
    onSurface: AppColors.textPrimary,
    onSurfaceVariant: AppColors.textSecondary,
    outline: AppColors.border,
    outlineVariant: AppColors.border,
    error: AppColors.danger,
  );
  final buttonShape = RoundedRectangleBorder(borderRadius: BorderRadius.circular(Radii.button));
  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: AppColors.background,
    fontFamily: 'Inter',
    textTheme: TextTheme(
      displayLarge: _text(48, 52, FontWeight.w600, letterSpacing: -1.2),
      displaySmall: _text(34, 40, FontWeight.w600, letterSpacing: -0.8),
      titleLarge: _text(22, 28, FontWeight.w600, letterSpacing: -0.3),
      titleMedium: _text(17, 24, FontWeight.w600, letterSpacing: -0.1),
      bodyLarge: _text(16, 24, FontWeight.w400),
      bodyMedium: _text(15, 22, FontWeight.w400),
      labelLarge: _text(14, 20, FontWeight.w500),
      bodySmall: _text(12, 16, FontWeight.w400, color: AppColors.textSecondary),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        minimumSize: const Size.fromHeight(56),
        shape: buttonShape,
        textStyle: _text(16, 20, FontWeight.w600),
        disabledBackgroundColor: AppColors.surfaceMuted,
        disabledForegroundColor: AppColors.textSecondary,
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        minimumSize: const Size.fromHeight(56),
        shape: buttonShape,
        foregroundColor: AppColors.textPrimary,
        side: const BorderSide(color: AppColors.border),
        textStyle: _text(16, 20, FontWeight.w600),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        minimumSize: const Size(48, 48),
        shape: buttonShape,
        textStyle: _text(15, 20, FontWeight.w600),
      ),
    ),
    bottomSheetTheme: const BottomSheetThemeData(
      backgroundColor: AppColors.surface,
      surfaceTintColor: Colors.transparent,
      showDragHandle: true,
      dragHandleColor: AppColors.border,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.vertical(top: Radius.circular(Radii.sheet))),
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: AppColors.surface,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(Radii.card)),
    ),
    dividerTheme: const DividerThemeData(color: AppColors.border, thickness: 1, space: 1),
  );
}

TextStyle _text(
  double size,
  double lineHeight,
  FontWeight weight, {
  double letterSpacing = 0,
  Color color = AppColors.textPrimary,
}) {
  return TextStyle(
    fontFamily: 'Inter',
    fontSize: size,
    height: lineHeight / size,
    fontWeight: weight,
    letterSpacing: letterSpacing,
    color: color,
  );
}
