// A bit's notes as a sheet (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §2). Phone: a native form sheet
// at the medium detent first (large on drag), the grabber, Close leading, the short name as the title (app/_layout.tsx);
// at the medium detent the stage behind is NOT dimmed, so the clock stays readable and the dock still works; keep-awake
// and the recorder keep running because the stage stays mounted under a presented route. The footer is the dock's own
// 60-pt control: on stage "Next bit" advances the stage AND this sheet (the sheet follows the current bit from then on;
// "Last bit" on the last one), from a set list it reads "Open the bit". Fed from lib/stageNotes: no fetch (the build
// already carries each bit's words). Setup and reveal in type.stageProse, the reveal heavier; the typed idea under
// "Idea" when there is no written version; the stage line last. Web: the RN Sheet with Close.
import React from 'react';
import { Platform, ScrollView, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { SideButton } from '@/lib/RecordControl';
import { useStageNotes } from '@/lib/stageNotes';
import type { Step } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { mmss } from '@/rules/format';
import { notesFooter, notesFor } from '@/rules/setListRules';
import { C, F } from '@/lib/theme';
import { Sheet, SheetHead, T } from '@/lib/ui';

const enc = encodeURIComponent;
const nameOf = (x: Step) => x.short_name || shortNameOf(x.title);

function Words({ step, i, n }: { step: Step; i: number; n: number }) {
  const w = notesFor(step);
  return (
    <View style={{ gap: 14 }}>
      <T v="eb" testID="notes-position">{i + 1} of {n}{step.duration_s ? ` · ${step.estimated ? '~' : ''}${mmss(step.duration_s)}` : ''}</T>
      {w.kind === 'empty' && <T v="mut" testID="notes-empty">No words saved for this bit yet.</T>}
      {!!w.setup && <T v="stageProse" testID="notes-setup">{w.setup}</T>}
      {!!w.reveal && <T v="stageProse" testID="notes-reveal" style={{ fontFamily: F.semi }}>{w.reveal}</T>}
      {!!w.idea && (
        <View style={{ gap: 4 }}>
          <T v="eb">Idea</T>
          <T v="stageProse" testID="notes-idea">{w.idea}</T>
        </View>
      )}
      {!!step.stage_line && <T v="mut" testID="notes-stage-line">{step.stage_line}</T>}
    </View>
  );
}

export default function BitNotesRoute() {
  const notes = useStageNotes();
  const ins = useSafeAreaInsets();
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
  const body = step ? <Words step={step} i={i} n={steps.length} /> : <T v="mut" testID="notes-none">Nothing to show here. Open a set list, or the stage, and tap a bit.</T>;
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
          <View style={{ paddingHorizontal: 16, paddingTop: 8, paddingBottom: Math.max(ins.bottom, 12), backgroundColor: C.surface }}>{footer}</View>
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
