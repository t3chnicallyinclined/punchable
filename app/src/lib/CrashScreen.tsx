// What he sees instead of a silent exit: the error, where it happened, the stack, and a way to get the text off the
// phone. Rendered by the root layout's ErrorBoundary export (expo-router wraps the root route in its Try boundary:
// expo-router/build/useScreens.js, fromImport) so a throw anywhere in the tree ends here, not in RCTFatal. No fonts,
// no providers, no theme context: this must draw when nothing else can. Colours are the app's dark ground.
import React, { useEffect, useMemo } from 'react';
import { Platform, Pressable, ScrollView, Share, Text, View } from 'react-native';
import { crashText, formatCrash } from './crashlog';
import { recordCrash } from './crashReport';

const BG = '#0B0B0D';
const CARD = '#1C1C1F';
const TEXT = '#f3f4f6';
const MUT = '#A1A1A8';
const AMBER = '#FFB340';

/** Web: the clipboard. Phone: the share sheet (its first row is Copy; expo-clipboard is not a dependency yet). */
export async function copyOrShare(text: string): Promise<'copied' | 'shared' | 'failed'> {
  try {
    if (Platform.OS === 'web') {
      await (globalThis as any).navigator?.clipboard?.writeText(text);
      return 'copied';
    }
    await Share.share({ message: text });
    return 'shared';
  } catch {
    return 'failed';
  }
}

export function CrashScreen({ error, retry, context }: { error: Error; retry: () => Promise<void> | void; context?: Record<string, string> }) {
  const rec = useMemo(() => formatCrash(error, 'boundary', context), [error, context]);
  useEffect(() => {
    recordCrash(error, 'boundary', context);
  }, [error, context]);
  const text = crashText(rec);
  return (
    <View testID="crash-screen" style={{ flex: 1, backgroundColor: BG }}>
      <ScrollView contentContainerStyle={{ padding: 24, paddingTop: 72, gap: 16 }}>
        <Text accessibilityRole="header" style={{ color: TEXT, fontSize: 24, fontWeight: '800' }}>Punchable hit a bug</Text>
        <Text style={{ color: MUT, fontSize: 15, lineHeight: 21 }}>The screen could not be drawn. The error is below; copy it and send it along. Nothing you wrote is lost.</Text>
        <View style={{ backgroundColor: CARD, borderRadius: 12, padding: 14 }}>
          <Text testID="crash-message" selectable style={{ color: TEXT, fontSize: 15, lineHeight: 21, fontWeight: '600' }}>
            {rec.name}: {rec.message}
          </Text>
          <Text selectable style={{ color: MUT, fontSize: 12, lineHeight: 17, marginTop: 10, fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace' }}>
            {rec.stack || '(no stack)'}
          </Text>
        </View>
        <Pressable testID="crash-copy" accessibilityRole="button" onPress={() => copyOrShare(text)}
          style={({ pressed }) => ({ minHeight: 52, borderRadius: 12, backgroundColor: AMBER, alignItems: 'center', justifyContent: 'center', opacity: pressed ? 0.85 : 1 })}>
          <Text style={{ color: BG, fontSize: 17, fontWeight: '700' }}>{Platform.OS === 'web' ? 'Copy' : 'Copy or share'}</Text>
        </Pressable>
        <Pressable testID="crash-retry" accessibilityRole="button" onPress={() => retry()}
          style={({ pressed }) => ({ minHeight: 52, borderRadius: 12, borderWidth: 1, borderColor: '#4b5059', alignItems: 'center', justifyContent: 'center', opacity: pressed ? 0.85 : 1 })}>
          <Text style={{ color: TEXT, fontSize: 17, fontWeight: '600' }}>Try again</Text>
        </Pressable>
      </ScrollView>
    </View>
  );
}
