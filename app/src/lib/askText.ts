// One short text asked for in place (Rename…): the system's alert with a field on iOS (Alert.prompt), the browser's
// prompt on the web. Resolves to the words, or null when he cancels or leaves it empty.
import { Alert, Platform } from 'react-native';

export function askText(title: string, initial: string, ok = 'Save'): Promise<string | null> {
  if (Platform.OS === 'ios') {
    return new Promise((resolve) => {
      Alert.prompt(title, undefined, [
        { text: 'Cancel', style: 'cancel', onPress: () => resolve(null) },
        { text: ok, onPress: (v?: string) => resolve((v || '').trim() || null) },
      ], 'plain-text', initial);
    });
  }
  const w = (globalThis as any).prompt as ((m: string, d: string) => string | null) | undefined;
  const v = w ? w(title, initial) : null;
  return Promise.resolve((v || '').trim() || null);
}
