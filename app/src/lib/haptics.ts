// Haptics (HIG): selection on reorder / tab change, light impact on lift and hold-to-talk, success on save,
// warning on destructive. No-op on the web and when the Settings toggle is off (a device preference, lib/prefs).
import { Platform } from 'react-native';
import { getPrefs } from './prefs';

let H: any = null;
if (Platform.OS !== 'web') {
  try {
    H = require('expo-haptics');
  } catch {
    H = null;
  }
}
const on = () => !!H && getPrefs().haptics;
const run = (f: () => Promise<void>) => {
  try {
    f().catch(() => {});
  } catch {
    /* no haptic engine (simulator) */
  }
};

export const haptics = {
  selection: () => on() && run(() => H.selectionAsync()),
  light: () => on() && run(() => H.impactAsync(H.ImpactFeedbackStyle.Light)),
  medium: () => on() && run(() => H.impactAsync(H.ImpactFeedbackStyle.Medium)),
  success: () => on() && run(() => H.notificationAsync(H.NotificationFeedbackType.Success)),
  warning: () => on() && run(() => H.notificationAsync(H.NotificationFeedbackType.Warning)),
  error: () => on() && run(() => H.notificationAsync(H.NotificationFeedbackType.Error)),
  available: Platform.OS !== 'web' && !!H,
};
