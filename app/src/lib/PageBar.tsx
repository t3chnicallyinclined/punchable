// The page's one bar (docs/design/SPEC-2026-11-17-one-notebook.md screens 2 and 3). It sits on the keyboard while he
// types (a plain view at the keyboard's top, the repo's keyboard-bar pattern; never an InputAccessoryView, which binds
// to its input once: journal 2026-11-17) and at the bottom of the page when the keyboard is down, and it has two
// faces of the same height:
//   - with a caret: Say it · Film it · Punch it up (the page's three actions; the third is the round's button and
//     says what it is right now: Get a new draft, Writing…, New draft ready);
//   - with words selected: Funnier · Shorter · 3 options · Note, for exactly those words. Each is a switch (lit while
//     the words are marked that way). React Native cannot add items to the system's selection bubble, so the bubble
//     is left alone and these sit where the thumb is.
// When he has marks and the keyboard is down it is the mockup's footer instead: the sentence and "Get a new draft".
// It holds the selection itself (the document tells it through `select`), so a caret moving as he types re-draws this
// bar and never the page. Every target is 44 pt and says its name; VoiceOver hears the bar change face once.
// Rules: rules/pageRules.ts.
import React, { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { PixelRatio, Pressable, Text, View } from 'react-native';
import { announce } from './announce';
import { BarGround } from './BarGround';
import { Icon } from './Icon';
import { C, F } from './theme';
import type { Mark } from '@/rules/markRules';
import { targetOf } from '@/rules/markRules';
import { ASKS, lit, markOn, marksLine, type Ask, type Punch } from '@/rules/pageRules';

/** The labels' Dynamic Type cap: four buttons fit one row on the narrowest iPhone. */
const BAR_CAP = 1.3;
/** The row of buttons with its padding; the one line above it when the page has something to say. */
export const PAGE_BAR_H = 56;
export const PAGE_NOTE_H = 22;

export type Target = { text: string; at: number };
export type PageBarHandle = { select: (sel: { start: number; end: number }, text: string) => void };
type Seed = () => { sel: { start: number; end: number }; text: string } | null;

/** What a mark made now would be on: only words he selected (a caret marks nothing; it shows the page's actions). */
function wordsOf(text: string, sel: { start: number; end: number } | null): Target | null {
  return sel && sel.start !== sel.end ? targetOf(text, sel) : null;
}

export const PageBar = forwardRef<PageBarHandle, {
  /** The marks on this page: the buttons light for the one on the selected words. */ marks: readonly Mark[];
  /** The document's text now (the marks are found in it by their words). */ text: () => string;
  /** The page's main button. */ punch: Punch;
  onSay: () => void; onFilm: () => void; onPunch: () => void;
  /** Film it has nothing to read, or this is the web (the camera is the phone's). */ filmOff?: string | null;
  /** A tap on one of the four with words selected. */ onAsk: (ask: Ask, target: Target) => void;
  /** One line above the buttons: why the words are not a version yet (offline, too long). */ note?: string | null;
  /** The keyboard is down and he has marks: the footer (the sentence, Get a new draft) instead of the three. */ footer?: boolean;
  /** The selection the document last reported, read once when the bar mounts (the tap that focused the document came
   *  before the bar existed). */ seed?: Seed;
  testID?: string;
}>(function PageBar(p, ref) {
  const [target, setTarget] = useState<Target | null>(() => {
    const s = p.seed?.();
    return s ? wordsOf(s.text, s.sel) : null;
  });
  useImperativeHandle(ref, () => ({
    select: (sel, text) => setTarget((was) => {
      const t = wordsOf(text, sel);
      return was && t && was.text === t.text && was.at === t.at ? was : t;
    }),
  }), []);
  // VoiceOver hears the bar change face, once per change and never on the first draw (HIG/voiceover: say layout
  // changes; a selection handle being dragged does not say it again)
  const face = target ? 'words' : 'page';
  const lastFace = useRef(face);
  useEffect(() => {
    if (lastFace.current === face) return;
    lastFace.current = face;
    const t = setTimeout(() => announce(face === 'words' ? 'Actions for the selected words' : 'Page actions'), 600);
    return () => clearTimeout(t);
  }, [face]);

  const note = !!p.note && (
    <Text testID={p.testID ? `${p.testID}-note` : undefined} numberOfLines={1} maxFontSizeMultiplier={BAR_CAP}
      style={{ height: PAGE_NOTE_H, lineHeight: PAGE_NOTE_H, paddingHorizontal: 14, fontFamily: F.semi, fontSize: 13, color: C.mut2, textAlign: 'center' }}>{p.note}</Text>
  );

  if (p.footer && !target) {
    // the mockup's footer: what will happen, and the one button
    const off = p.punch.does === 'none';
    return (
      <BarGround testID={p.testID}>
        <View style={{ paddingHorizontal: 20, paddingTop: 10, paddingBottom: 8, gap: 10 }}>
          <Text testID={p.testID ? `${p.testID}-line` : undefined} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 19, color: C.mut2, textAlign: 'center' }}>
            {p.note || marksLine(p.marks.length)}
          </Text>
          <Pressable testID={p.testID ? `${p.testID}-punch` : undefined} accessibilityRole="button" accessibilityLabel={p.punch.label} accessibilityHint={p.punch.hint}
            accessibilityState={{ disabled: off }} disabled={off} onPress={p.onPunch}
            style={({ pressed }) => ({ minHeight: 50, borderRadius: 14, alignItems: 'center', justifyContent: 'center', backgroundColor: C.amber, opacity: off ? 0.45 : pressed ? 0.85 : 1 })}>
            <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 17, color: C.bg }}>{p.punch.label}</Text>
          </Pressable>
        </View>
      </BarGround>
    );
  }

  if (target) {
    const on = lit(markOn(p.text(), p.marks as Mark[], target));
    return (
      <BarGround testID={p.testID}>
        {note}
        <View accessibilityRole="toolbar" style={{ height: PAGE_BAR_H, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 8 }}>
          {ASKS.map((a) => (
            <Pressable key={a.k} testID={p.testID ? `${p.testID}-${a.k}` : undefined} accessibilityRole="button" accessibilityLabel={a.a11y} accessibilityHint={a.hint}
              accessibilityState={{ selected: on[a.k] }} onPress={() => p.onAsk(a.k, target)}
              style={({ pressed }) => ({
                flex: 1, minHeight: 44, borderRadius: 10, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 4,
                // lit = marked that way: a filled ground and a check, never colour alone (HIG/color)
                backgroundColor: on[a.k] ? C.amber : pressed ? C.card2 : C.card,
              })}>
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, flexShrink: 1 }}>
                {on[a.k] && <Icon name="check" size={14} color={C.bg} sw={2.6} />}
                <Text numberOfLines={1} adjustsFontSizeToFit minimumFontScale={0.85} maxFontSizeMultiplier={BAR_CAP}
                  style={{ flexShrink: 1, fontFamily: F.semi, fontSize: 15, color: on[a.k] ? C.bg : a.k === 'note' ? C.text : C.amber }}>{a.label}</Text>
              </View>
            </Pressable>
          ))}
        </View>
      </BarGround>
    );
  }

  const punchOff = p.punch.does === 'none';
  // at the larger text sizes the two side buttons are their symbols alone (their names stay for VoiceOver), so the
  // main button's words always fit (HIG/typography: never truncate the one action)
  const tight = PixelRatio.getFontScale() >= 1.3;
  const side = (testID: string, label: string, icon: 'mic' | 'video', onPress: () => void, off?: string | null) => (
    <Pressable testID={p.testID ? `${p.testID}-${testID}` : undefined} accessibilityRole="button" accessibilityLabel={label} accessibilityHint={off || undefined}
      accessibilityState={{ disabled: !!off }} disabled={!!off} onPress={onPress}
      style={({ pressed }) => ({ minHeight: 44, paddingHorizontal: 12, borderRadius: 10, flexDirection: 'row', alignItems: 'center', gap: 6, backgroundColor: pressed ? C.card2 : 'transparent', opacity: off ? 0.45 : 1 })}>
      {/* the label colour, never red: red is "recording now" (HIG/color: one meaning per colour) */}
      <Icon name={icon} size={20} color={C.text} />
      {!tight && <Text numberOfLines={1} maxFontSizeMultiplier={BAR_CAP} style={{ fontFamily: F.body, fontSize: 15, color: C.text }}>{label}</Text>}
    </Pressable>
  );
  return (
    <BarGround testID={p.testID}>
      {note}
      <View accessibilityRole="toolbar" style={{ height: PAGE_BAR_H, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', paddingHorizontal: 10 }}>
        {side('say', 'Say it', 'mic', p.onSay)}
        {side('film', 'Film it', 'video', p.onFilm, p.filmOff)}
        <Pressable testID={p.testID ? `${p.testID}-punch` : undefined} accessibilityRole="button" accessibilityLabel={p.punch.label} accessibilityHint={p.punch.hint}
          accessibilityState={{ disabled: punchOff }} disabled={punchOff} onPress={p.onPunch}
          style={({ pressed }) => ({ minHeight: 44, flexShrink: 1, paddingHorizontal: 16, borderRadius: 22, flexDirection: 'row', alignItems: 'center', gap: 6, backgroundColor: C.amber, opacity: punchOff ? 0.45 : pressed ? 0.85 : 1 })}>
          <Icon name="spark" size={18} color={C.bg} />
          <Text numberOfLines={1} adjustsFontSizeToFit minimumFontScale={0.85} maxFontSizeMultiplier={BAR_CAP} style={{ flexShrink: 1, fontFamily: F.semi, fontSize: 15, color: C.bg }}>{p.punch.label}</Text>
        </Pressable>
      </View>
    </BarGround>
  );
});
