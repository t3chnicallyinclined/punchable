// Device checks: the questions only his phone can answer, each with what to do and what pass looks like. Reached from
// Settings › About › Device checks. Sentences are allowed here: it is an instrument, not a screen.
//
//   1-3  the bit page (SPEC 2026-11-03 §4.1; still unanswered): the bottom toolbar against the tab bar (this page),
//        the glass bar above the keyboard and dictation into per-line inputs (/spike-write).
//   4    where Record sits (SPEC 2026-11-16 §4.1, §7 Q1): a strip above the tab bar (the bottom accessory), a button
//        at the end of the tab bar (the trailing slot), a button in the top bar (this page's own). The switches are
//        lib/deviceChecks: memory only, off at every launch. No test control records anything.
//   5    highlighting words in a line and our own marks row (§4.2): /spike-select.
//   6    Backspace at the start of a line, the Join up key, the caret after a split and a join (§4.3): /spike-write.
//   7    recording while the walk-through shows, then the camera on top, muted and unmuted (§4.4): /spike-run.
//   8-9  two things seen in passing (§5 R11): whether the conversation's input bar is drawn on this phone, and
//        whether localStorage survives closing the app (several small stores lean on it).
//
// This page scrolls in the Bits stack so the system tab bar is beneath and minimises (check 1 and check 4 need that).
// Removed, with its three sibling pages, the tab shell's two test controls and (tabs)/recslot, once the answers are in.
import React, { useState } from 'react';
import { Platform, Switch, Text, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { ACCESSORY } from '@/lib/accessory';
import { deviceChecks, useDeviceChecks } from '@/lib/deviceChecks';
import { useFlags } from '@/lib/useFlags';
import { C, F } from '@/lib/theme';
import { Btn, Card, T, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const native = Platform.OS !== 'web';
const LS_KEY = 'pk.devcheck.ls';

const BIT_PAGE: [string, string, string][] = [
  ['1 · Bottom toolbar vs the tab bar', 'Scroll down (the tab bar shrinks), scroll up. Tap Write below. Then Back to Bits.',
    'Pass: Write floats above the tab bar in both states and is tappable (a message pops up at the bottom); the footnote line sits just above Write; no empty bar is left on Bits after Back.'],
  ['2 · The glass bar above the keyboard', 'Open the line editor mock. Tap a line (keyboard up), move through the lines, hide the keyboard, bring it back.',
    'Pass: a 56-pt glass bar sits flush on the keyboard with no gap or overlap, moves with it, and returns to the bottom edge on dismiss.'],
  ['3 · Dictation into per-line inputs', 'In the line editor mock press the keyboard’s mic, say “line one, new line, line two”, stop. Then dictate and tap another line mid-sentence.',
    'Pass: the words land in the focused line; “new line” becomes a new line (the log says which way it arrived); Return between typed lines moves the caret without a keyboard flash.'],
];

let probed: string | null = null;
/** What localStorage held when this page first opened in this launch, then a fresh stamp written for the next
 *  launch. Once per launch: opening the page twice must not read back the stamp it has just written. */
function storageProbe(): string {
  if (probed != null) return probed;
  const ls = (globalThis as any).localStorage;
  if (!ls) return (probed = 'This phone cannot keep small settings (it has no localStorage): they are gone when the app closes.');
  let before: string | null = null;
  try { before = ls.getItem(LS_KEY); } catch { before = null; }
  const now = new Date().toISOString().slice(0, 19).replace('T', ' ');
  try { ls.setItem(LS_KEY, now); } catch { return (probed = 'This phone refuses to keep small settings (localStorage will not take a write).'); }
  return (probed = before ? `Found a stamp from ${before} UTC. Written now: ${now} UTC.` : `No stamp found. Written now: ${now} UTC.`);
}

function SwitchRow({ label, value, onChange, testID, disabled }: { label: string; value: boolean; onChange: (v: boolean) => void; testID: string; disabled?: boolean }) {
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12, minHeight: 44 }}>
      <Text maxFontSizeMultiplier={1.6} style={{ flex: 1, fontFamily: F.body, fontSize: 16, lineHeight: 22, color: disabled ? C.dim : C.text }}>{label}</Text>
      <Switch testID={testID} accessibilityLabel={label} value={value} onValueChange={onChange} disabled={disabled} />
    </View>
  );
}

export default function SpikePage() {
  const ui = useUI();
  const checks = useDeviceChecks();
  const flags = useFlags();
  const [stored] = useState(storageProbe);
  const [topBar, setTopBar] = useState(false);
  return (
    <Screen title="Device checks" back testID="spike" offlineAt="bottom" footnote="A footnote line sits here, above Write">
      {native && (
        <Stack.Toolbar placement="bottom">
          <Stack.Toolbar.Spacer />
          <Stack.Toolbar.Button variant="prominent" accessibilityLabel="Write" onPress={() => ui.snack('Write tapped: the bottom toolbar works here')}>Write</Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      {native && topBar && (
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button icon="record.circle" tintColor={C.red} accessibilityLabel="Record (a test control: nothing is recorded)" onPress={() => deviceChecks.tapped('the top bar')} />
        </Stack.Toolbar>
      )}
      <T v="mut">Nine checks for the next builds. Each says what to do and what pass looks like; tell us which way each went, one line each.</T>

      <T v="eb" accessibilityRole="header">The bit page</T>
      {BIT_PAGE.map(([title, doIt, pass]) => (
        <Card key={title} style={{ gap: 6 }}>
          <T v="semi">{title}</T>
          <T v="mut">{doIt}</T>
          <T v="small">{pass}</T>
        </Card>
      ))}

      <T v="eb" accessibilityRole="header">Where Record sits</T>
      <Card style={{ gap: 6 }} testID="check-record">
        <T v="semi">4 · Three places for a Record button</T>
        <T v="mut">Switch one on, look at it here, on Bits, on Sets and on a bit’s page, scroll so the tab bar shrinks, and tap it. None of them records: a tap only counts below.</T>
        <SwitchRow testID="check-accessory" label="A strip above the tab bar" value={checks.accessory} disabled={!native} onChange={(v) => deviceChecks.set({ accessory: v, accessoryHidden: false })} />
        <SwitchRow testID="check-accessory-hidden" label="Hide the strip (as a bit’s page would)" value={checks.accessoryHidden} disabled={!native || !checks.accessory} onChange={(v) => deviceChecks.set({ accessoryHidden: v })} />
        <SwitchRow testID="check-topbar" label="A button in the top bar (this page)" value={topBar} disabled={!native} onChange={setTopBar} />
        <SwitchRow testID="check-recordtab" label="A button at the end of the tab bar · the tabs reload when this changes" value={checks.recordTab} disabled={!native} onChange={(v) => deviceChecks.set({ recordTab: v })} />
        <T v="semi" testID="check-record-taps">Taps: {checks.taps}{checks.last ? ` · last: ${checks.last}` : ''}</T>
        <T v="small">Pass for the strip: it shows on all three tabs and on pushed pages, stays (smaller) when the tab bar shrinks, the hide switch removes it and brings it back, and on a bit’s page it does not cover Write. Pass for the end of the tab bar: a Record button that is not dimmed, a tap counts once and no search field opens. Tell us which one you would reach for with one thumb. Closing the app switches all of this off.</T>
      </Card>

      <T v="eb" accessibilityRole="header">Lines</T>
      <Card style={{ gap: 6 }}>
        <T v="semi">5 · Highlighting words in a line</T>
        <T v="mut">Open the selection mock. Press and hold a word, drag the handles, tap a line once.</T>
        <T v="small">Pass: press and hold selects words; a row under the line names exactly those words; a short tap still opens the line. Say whether the Copy bubble shows and whether it is in the way, with the switch on and off.</T>
        <Btn testID="spike-open-select" label="Open the selection mock" onPress={() => router.push('/spike-select' as any)} />
      </Card>
      <Card style={{ gap: 6 }}>
        <T v="semi">6 · Backspace joins two lines</T>
        <T v="mut">Open the line editor mock. Put the cursor at the start of line 2 and press Backspace. Do the same on the empty last line. Press Return in the middle of a line. Try the Join up key.</T>
        <T v="small">Pass: Backspace at the start joins the line to the one above, the cursor lands at the join, and no word is lost; Return splits at the cursor and the cursor starts the new line. If Backspace does nothing at the start of a line, say so: the log shows whether the key was reported, and Join up does the same job.</T>
        <Btn testID="spike-open-write" label="Open the line editor mock (checks 2, 3 and 6)" v="primary" onPress={() => router.push('/spike-write' as any)} />
      </Card>

      <T v="eb" accessibilityRole="header">Recording</T>
      <Card style={{ gap: 6 }}>
        <T v="semi">7 · Recording while you walk through a set</T>
        <T v="mut">Open the walk-through mock. Start the sound take, talk, tap Next a few times. Turn the camera on (muted), keep talking, then unmute it, keep talking, then Stop and check.</T>
        <T v="small">Pass: the clock and the level keep moving through Next and through the camera; after Stop the file is whole. The page says what happened at each step; tell us the last lines of its log. The microphone and camera indicators both show: that is expected.</T>
        <Btn testID="spike-open-run" label="Open the walk-through mock" disabled={!native} onPress={() => router.push('/spike-run' as any)} />
      </Card>

      <T v="eb" accessibilityRole="header">Seen in passing</T>
      <Card style={{ gap: 6 }} testID="check-saybar">
        <T v="semi">8 · Is the conversation’s input bar drawn?</T>
        <T v="mut">Bits › search for anything › ask it as a question, so a conversation opens. Is there a field to type the next question?</T>
        <T v="small">On this phone: Liquid Glass {ACCESSORY ? 'yes' : 'no'}; the server setting for the strip is {flags.glassAccessory ? 'on' : 'off'}. {ACCESSORY && flags.glassAccessory ? 'So the screen leaves the bar to the tab bar’s accessory, which nothing draws: we expect NO field.' : 'So the screen draws the bar itself: we expect a field.'} Tell us what you see.</T>
        <Btn testID="spike-open-say" label="Open Ask" onPress={() => router.push('/ask' as any)} />
      </Card>
      <Card style={{ gap: 6 }} testID="check-storage">
        <T v="semi">9 · Does the phone keep small settings?</T>
        <T v="mut">Read the line below. Close the app completely (swipe it away), open it and come back to this page.</T>
        <T v="semi" testID="check-storage-line">{stored}</T>
        <T v="small">Pass: after reopening, the line says it found the stamp you saw before. If it says “No stamp found” every time, the current set list and a few other choices are forgotten on every launch.</T>
      </Card>

      <T v="eb">Filler so the page scrolls</T>
      {Array.from({ length: 30 }).map((_, i) => (
        <View key={i} style={{ minHeight: 44, justifyContent: 'center', borderBottomWidth: 1, borderBottomColor: C.hairline }}>
          <Text style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.mut2 }}>Line {i + 1} of 30 · scroll past the tab bar and back</Text>
        </View>
      ))}
    </Screen>
  );
}
