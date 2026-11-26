import 'package:material_ui/material_ui.dart';

import 'tokens.dart';

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
