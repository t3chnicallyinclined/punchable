// An action sheet (HIG/action-sheets: "Use an action sheet — not an alert — to offer choices related to an intentional
// action"). Destructive choices go first ("at the top of the action sheet where they tend to be most noticeable"),
// Cancel last. iOS: the system sheet (ActionSheetIOS). Android: an alert with the same buttons. Web: a screen keeps its
// own sheet; this falls back to the browser's confirm for the one main choice.
import { ActionSheetIOS, Alert, Platform } from 'react-native';
import { C } from './theme';

export type SheetOption = { label: string; onPress?: () => void; destructive?: boolean; cancel?: boolean };

/** The order the HIG asks for: destructive, then the rest, then the cancel role. */
export function orderOptions(options: SheetOption[]): SheetOption[] {
  return [...options.filter((o) => o.destructive), ...options.filter((o) => !o.destructive && !o.cancel), ...options.filter((o) => o.cancel && !o.destructive)];
}

export function actionSheet({ title, message, options }: { title?: string; message?: string; options: SheetOption[] }) {
  const ordered = orderOptions(options);
  const destructive = ordered.map((o, i) => (o.destructive ? i : -1)).filter((i) => i >= 0);
  const cancel = ordered.findIndex((o) => o.cancel);
  if (Platform.OS === 'ios') {
    ActionSheetIOS.showActionSheetWithOptions(
      { title, message, options: ordered.map((o) => o.label), destructiveButtonIndex: destructive.length ? destructive : undefined, cancelButtonIndex: cancel >= 0 ? cancel : undefined, userInterfaceStyle: C.scheme },
      (i) => ordered[i]?.onPress?.(),
    );
    return;
  }
  if (Platform.OS === 'android') {
    Alert.alert(title || '', message, ordered.map((o) => ({ text: o.label, style: o.destructive ? 'destructive' : o.cancel ? 'cancel' : 'default', onPress: o.onPress })));
    return;
  }
  const main = ordered.find((o) => !o.cancel);
  const confirm = (globalThis as any).confirm as undefined | ((s: string) => boolean);
  if (main && (!confirm || confirm(`${title || ''}${message ? `\n${message}` : ''}\n\n${main.label}?`))) main.onPress?.();
  else ordered.find((o) => o.cancel)?.onPress?.();
}
