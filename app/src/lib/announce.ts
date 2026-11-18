// Say something to VoiceOver once (iOS has no live region: accessibilityLiveRegion is Android only). Queued, so it
// waits for what VoiceOver is already reading instead of cutting it off. The web's AccessibilityInfo has no
// `announceForAccessibilityWithOptions` at all: calling it there throws (in a timer or an effect, where nothing
// catches it), so every screen says things through this one function. tests/notebookShell.test.mjs pins that no
// shared file names the method itself.
import { AccessibilityInfo } from 'react-native';

export function announce(text: string | null | undefined): void {
  if (!text) return;
  try {
    if (AccessibilityInfo.announceForAccessibilityWithOptions) AccessibilityInfo.announceForAccessibilityWithOptions(text, { queue: true });
    else AccessibilityInfo.announceForAccessibility?.(text);
  } catch {
    /* saying it is never worth a crash */
  }
}
