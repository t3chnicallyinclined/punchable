// Device checks (SPEC 2026-11-03 §4.1): a spike screen the Slice 1 build ships for the owner, the only tester, and
// Slice 2 removes. Check 1 lives here: a scrolling page in the Bits stack (so the system tab bar is beneath and
// minimises on scroll) with the native bottom toolbar's one prominent button, and the footnote line the bit page pins
// above it. Checks 2 and 3 are the focus-mode mock (/spike-write), a full-screen modal like the real focus mode.
// Reached from Settings › About › Device checks. Sentences are allowed here: it is an instrument, not a screen.
import React from 'react';
import { Platform, Text, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { C, F } from '@/lib/theme';
import { Btn, Card, T, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const native = Platform.OS !== 'web';

const CHECKS: [string, string, string][] = [
  ['1 · Bottom toolbar vs the tab bar', 'Scroll down (the tab bar shrinks), scroll up. Tap Write below. Then Back to Bits.',
    'Pass: Write floats above the tab bar in both states and is tappable (a snack says so); the footnote line sits just above Write; no empty bar is left on Bits after Back.'],
  ['2 · The glass bar above the keyboard', 'Open the focus mock. Tap a line (keyboard up), move through the lines, hide the keyboard, bring it back.',
    'Pass: a 56-pt glass bar sits flush on the keyboard with no gap or overlap, moves with it, and returns to the bottom edge on dismiss.'],
  ['3 · Dictation into per-line inputs', 'In the focus mock press the keyboard’s mic, say “line one, new line, line two”, stop. Then dictate and tap another line mid-sentence.',
    'Pass: the words land in the focused line; “new line” becomes a new line (the log says which way it arrived); Return between typed lines moves the caret without a keyboard flash.'],
];

export default function SpikePage() {
  const ui = useUI();
  return (
    <Screen title="Device checks" back testID="spike" offlineAt="bottom" footnote="A footnote line sits here, above Write">
      {native && (
        <Stack.Toolbar placement="bottom">
          <Stack.Toolbar.Spacer />
          <Stack.Toolbar.Button variant="prominent" accessibilityLabel="Write" onPress={() => ui.snack('Write tapped: the bottom toolbar works here')}>Write</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      <T v="mut">Three checks for the next bit page. Each says what to do and what pass looks like; tell us which way each went.</T>
      {CHECKS.map(([title, doIt, pass]) => (
        <Card key={title} style={{ gap: 6 }}>
          <T v="semi">{title}</T>
          <T v="mut">{doIt}</T>
          <T v="small">{pass}</T>
        </Card>
      ))}
      <Btn testID="spike-open-write" label="Open the focus mock (checks 2 and 3)" v="primary" onPress={() => router.push('/spike-write' as any)} />
      <T v="eb">Filler so the page scrolls</T>
      {Array.from({ length: 40 }).map((_, i) => (
        <View key={i} style={{ minHeight: 44, justifyContent: 'center', borderBottomWidth: 1, borderBottomColor: C.hairline }}>
          <Text style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.mut2 }}>Line {i + 1} of 40 · scroll past the tab bar and back</Text>
        </View>
      ))}
    </Screen>
  );
}
