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

abstract final class Space {
  static const double xs = 4, s = 8, m = 12, l = 16, xl = 24, xxl = 32, xxxl = 48;
}

abstract final class Radii {
  static const double button = 12, card = 20, sheet = 28;
}

const tabularFigures = [FontFeature.tabularFigures()];
