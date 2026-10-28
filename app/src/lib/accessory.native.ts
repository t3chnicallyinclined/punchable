// Whether the input bar rides the tab bar as its bottom accessory (NativeTabs.BottomAccessory, the MiniPlayer
// pattern: HIG/tab-bars "you can choose to minimize the tab bar and move the accessory inline with it when a person
// scrolls down"). Only where the system draws Liquid Glass (iOS 26+); below that, and on Android, the bar stays an
// absolute dock above the tab bar (lib/Screen.native).
import { Platform } from 'react-native';
import { isLiquidGlassAvailable } from 'expo-glass-effect';

let glass = false;
try {
  glass = Platform.OS === 'ios' && isLiquidGlassAvailable();
} catch {
  glass = false;
}
export const ACCESSORY = glass;
