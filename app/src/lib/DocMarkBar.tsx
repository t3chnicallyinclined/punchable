// The marks bar of the bit's document (SPEC 2026-11-16 Slice 3): it holds what a mark would be on (the selection the
// document reports) itself, so a caret moving as he types re-draws this bar and never the page. The page puts it on the
// keyboard while he types (a plain view at the keyboard's top, as lib/KeyboardBar; not an InputAccessoryView, which
// binds to its input only once and is deprecated in RN 0.88: iOS dev review), and in the shell's bottom bar on the web.
// A mark made here says so in the bar for a few seconds with Undo, and to VoiceOver (accessibilityLiveRegion is
// Android only: AccessibilityInfo.announceForAccessibility, as lib/TopBar does).
import React, { forwardRef, useEffect, useImperativeHandle, useState } from 'react';
import { AccessibilityInfo } from 'react-native';
import { MarkBar, type BarAction } from './MarkBar';
import { targetOf } from '@/rules/markRules';

export type DocMarkBarHandle = { select: (sel: { start: number; end: number }, text: string) => void };
/** What the page says back after a mark: a word for the bar, and the way to take the mark back. */
export type MarkSaid = { word: string; undo?: () => void } | null;
const FLASH_MS = 4000;

export const DocMarkBar = forwardRef<DocMarkBarHandle, {
  count: number;
  /** Make the mark; a word back ("Kept", or why not), with an Undo when a mark was made. */
  onAct: (a: BarAction, target: { text: string; at: number }) => MarkSaid;
  onTray: () => void;
  /** The page's footnote line (offline, saved as v9), shown in the bar while it covers the page's own. */ note?: string | null;
  offline?: boolean;
  /** The selection the document last reported, read once when the bar mounts: the tap that focused the document came
   *  before the bar existed (iOS dev review). */ seed?: () => { sel: { start: number; end: number }; text: string } | null;
  testID?: string;
}>(function DocMarkBar(p, ref) {
  const [target, setTarget] = useState<{ text: string; at: number } | null>(() => {
    const s = p.seed?.();
    return s ? targetOf(s.text, s.sel) : null;
  });
  // with VoiceOver on, a word stays until his next mark: 4 s is too short to hear it and reach Undo (design review)
  const [reader, setReader] = useState(false);
  useEffect(() => { void AccessibilityInfo.isScreenReaderEnabled().then(setReader).catch(() => {}); }, []);
  const [said, setSaid] = useState<MarkSaid>(null);
  useImperativeHandle(ref, () => ({
    select: (sel, text) => setTarget((was) => {
      const t = targetOf(text, sel);
      return was && t && was.text === t.text && was.at === t.at ? was : t;
    }),
  }), []);
  useEffect(() => {
    if (!said) return;
    AccessibilityInfo.announceForAccessibilityWithOptions(said.word, { queue: true });
    if (reader) return;
    const t = setTimeout(() => setSaid(null), FLASH_MS);
    return () => clearTimeout(t);
  }, [said, reader]);
  return (
    <MarkBar testID={p.testID} target={target} count={p.count} flash={said?.word} note={p.note} offline={p.offline} onTray={p.onTray}
      onUndo={said?.undo ? () => { said.undo?.(); setSaid({ word: 'Mark removed' }); } : null}
      onAct={(a) => { if (target) setSaid(p.onAct(a, target)); }} />
  );
});
