import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';
import 'package:nearvolt/theme.dart';

/// Text must be readable: 4.5:1 is the minimum for small text.
void main() {
  test('the grey help text reads on the page and on cards', () {
    expect(contrastRatio(AppColors.textSecondary, AppColors.background), greaterThanOrEqualTo(4.5));
    expect(contrastRatio(AppColors.textSecondary, AppColors.surface), greaterThanOrEqualTo(4.5));
    expect(contrastRatio(AppColors.textPrimary, AppColors.background), greaterThanOrEqualTo(7));
    expect(contrastRatio(AppColors.accent, AppColors.accentSoft), greaterThanOrEqualTo(4.5));
  });

  test('status colours are darkened to read on their own tint', () {
    for (final color in [AppColors.success, AppColors.warning, AppColors.danger, AppColors.accent]) {
      final fill = Color.alphaBlend(color.withValues(alpha: 0.1), AppColors.background);
      final text = readableOn(color, fill);
      expect(contrastRatio(text, fill), greaterThanOrEqualTo(4.5), reason: 'text in $color');
    }
    // A colour that is already dark enough is left alone.
    expect(readableOn(AppColors.textPrimary, AppColors.background), AppColors.textPrimary);
  });
}
