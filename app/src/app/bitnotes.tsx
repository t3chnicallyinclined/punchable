// A bit's notes as a sheet (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §2). Phone: a native form sheet
// at the medium detent first (large on drag), the grabber, Close leading, the short name as the title (app/_layout.tsx);
// at the medium detent the stage behind is NOT dimmed, so the clock stays readable and the dock still works; keep-awake
// and the recorder keep running because the stage stays mounted under a presented route. The footer is the dock's own
// 60-pt control: on stage "Next bit" advances the stage AND this sheet (the sheet follows the current bit from then on;
// "Last bit" on the last one), from a set list it reads "Open the bit". Fed from lib/stageNotes: no fetch (the build
// already carries each bit's words). Setup and reveal in type.stageProse, the reveal heavier; the typed idea under
// "Idea" when there is no written version; the stage line last. Since Slice 4 (SPEC 2026-11-16 §3.3) his own bullet
// notes for the bit in this set lead, at stage size, and the bit's words follow under "The bit" (folded behind that
// row when the sheet is opened from the stage). Web: the RN Sheet with Close.
import React, { useState } from 'react';
import { Platform, Pressable, ScrollView, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { SideButton } from '@/lib/RecordControl';
import { useStageNotes } from '@/lib/stageNotes';
import type { Step } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { mmss } from '@/rules/format';
import { footerBottomPad } from '@/rules/layoutRules';
import { notesFooter, notesFor } from '@/rules/setListRules';
import { hisNotes } from '@/rules/setNoteRules';
import { C, F } from '@/lib/theme';
import { Icon, Sheet, SheetHead, T, useBottomInset } from '@/lib/ui';

const enc = encodeURIComponent;
const nameOf = (x: Step) => x.short_name || shortNameOf(x.title);

function Words({ step, i, n, fromStage }: { step: Step; i: number; n: number; fromStage: boolean }) {
  const w = notesFor(step);
  // his own bullet notes for this bit in this set come first, at stage size (SPEC 2026-11-16 §3.3); the bit's words
  // follow under "The bit", smaller, for when he wants the wording. On stage they stay folded behind that row until
  // he asks (his notes, not the app's split of the bit, are what he reads there). With none of his, the words lead
  // as before.
  const mine = hisNotes(step);
  const prose = mine.length ? 'body' : 'stageProse';
  const fold = fromStage && mine.length > 0 && w.kind !== 'empty';
  const [open, setOpen] = useState(false);
  const words = !fold || open;
  return (
    <View style={{ gap: 14 }}>
      <T v="eb" testID="notes-position">{i + 1} of {n}{step.duration_s ? ` · ${step.estimated ? '~' : ''}${mmss(step.duration_s)}` : ''}</T>
      {mine.length > 0 && (
        <View testID="notes-his" accessibilityLabel={`Your notes: ${mine.join('. ')}`} accessible style={{ gap: 10 }}>
          {mine.map((t, j) => (
            <View key={j} style={{ flexDirection: 'row', gap: 10 }}>
              <T v="stageProse" accessible={false} style={{ color: C.mut2 }}>•</T>
              <T v="stageProse" testID={`notes-his-${j}`} style={{ flex: 1, minWidth: 0 }}>{t}</T>
            </View>
          ))}
        </View>
      )}
      {w.kind === 'empty' && !mine.length && <T v="mut" testID="notes-empty">No words saved for this bit yet.</T>}
      {fold && (
        <Pressable testID="notes-thebit" accessibilityRole="button" accessibilityState={{ expanded: open }} accessibilityLabel="The bit" accessibilityHint={open ? 'Hides the bit’s words' : 'Shows the bit’s words'}
          onPress={() => setOpen(!open)} style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44, opacity: pressed ? 0.6 : 1 })}>
          <T v="eb" style={{ flex: 1 }}>The bit</T>
          <Icon name={open ? 'chevronDown' : 'chevron'} size={14} color={C.dim} />
        </Pressable>
      )}
      {!fold && mine.length > 0 && w.kind !== 'empty' && <T v="eb" accessibilityRole="header">The bit</T>}
      {words && !!w.setup && <T v={prose} testID="notes-setup" style={mine.length ? { color: C.text2 } : undefined}>{w.setup}</T>}
      {words && !!w.reveal && <T v={prose} testID="notes-reveal" style={{ fontFamily: F.semi, ...(mine.length ? { color: C.text2 } : null) }}>{w.reveal}</T>}
      {words && !!w.idea && (
        <View style={{ gap: 4 }}>
          {!mine.length && <T v="eb">Idea</T>}
          <T v={prose} testID="notes-idea" style={mine.length ? { color: C.text2 } : undefined}>{w.idea}</T>
        </View>
      )}
      {!!step.stage_line && <T v="mut" testID="notes-stage-line">{step.stage_line}</T>}
    </View>
  );
}

export default function BitNotesRoute() {
  const notes = useStageNotes();
  // the root provider's inset (this route is not inside lib/Screen): the sheet's own window, the home indicator alone;
  // react-native-screens lays unstable_sheetFooter flush with the sheet's bottom edge and adds no inset of its own
  const bottom = useBottomInset();
  const close = () => (router.canGoBack() ? router.back() : router.navigate('/sets' as any));
  const steps = notes?.steps || [];
  const i = Math.max(0, Math.min(steps.length - 1, notes?.index ?? 0));
  const step: Step | undefined = steps[i];
  const title = step ? nameOf(step) : 'Notes';
  const fromStage = !!notes?.onNext;
  const foot = notesFooter(fromStage, i, steps.length);
  const after = steps[i + 1];
  const footLabel = fromStage && after && !foot.disabled ? `${foot.label}, ${nameOf(after)}` : foot.label;
  const act = () => {
    if (fromStage) notes?.onNext?.();
    else if (step) { close(); router.push(`/joke/${enc(step.joke_id)}` as any); }
  };
  const body = step ? <Words key={step.joke_id} step={step} i={i} n={steps.length} fromStage={fromStage} /> : <T v="mut" testID="notes-none">Nothing to show here. Open a set list, or the stage, and tap a bit.</T>;
  const footer = <SideButton testID="notes-next" prominent label={foot.label} accessibilityLabel={footLabel} disabled={foot.disabled} onPress={act} />;
  if (Platform.OS === 'web') {
    return (
      <View style={{ flex: 1, backgroundColor: C.bg }}>
        <Sheet visible onClose={close} label={title} testID="notes-sheet">
          <SheetHead title={title} onCancel={close} cancelLabel="Close" testID="notes" />
          {body}
          <View style={{ marginTop: 16 }}>{footer}</View>
        </Sheet>
      </View>
    );
  }
  return (
    <View testID="notes-sheet" style={{ flex: 1, backgroundColor: C.surface }}>
      <Stack.Screen options={{
        title,
        // the sheet's own footer: pinned under the content at every detent (react-native-screens ScreenFooter)
        unstable_sheetFooter: () => (
          <View style={{ paddingHorizontal: 16, paddingTop: 8, paddingBottom: footerBottomPad(bottom), backgroundColor: C.surface }}>{footer}</View>
        ),
      }} />
      {/* HIG/sheets: Close on the leading edge; no Done (nothing to save) */}
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button accessibilityLabel="Close" onPress={close}>Close</Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ padding: 16, paddingBottom: 96 }}>
        {body}
      </ScrollView>
    </View>
  );
}
