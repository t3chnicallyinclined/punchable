// The bit as a document (owner 2026-11-16: "a bit is a document ... it should be as easy as editing a google doc on
// your phone"; SPEC 2026-11-16 B11, §4.3, Slice 2). ONE multiline input over the whole text, sitting in the page's own
// scroll view: a tap anywhere puts the caret there and brings up the keyboard; selection across lines, paste,
// dictation and Backspace joining two lines are the system's, as in Notes. There is no Write screen to enter and no
// Save to press:
//   - a moment after every keystroke the words are written to the phone (lib/bitDraft), so nothing typed is lost
//     offline or if the app dies;
//   - a VERSION is saved when he leaves the text (Done, a tap elsewhere), leaves the page, sends the app to the
//     background, pauses (rules/lineRules PAUSE_MS, at most once per PAUSE_GAP_MS), or gets his connection back; only
//     when the words changed, never an empty one. One save at a time: a second asks again when the first has settled;
//   - offline the draft waits on the phone and is sent when the connection returns (here, or by flushBitDrafts); a
//     save that failed while online is tried again by itself.
// The input is uncontrolled (the text view owns the text; React never writes it back mid-word, so the caret, the
// selection, autocorrect and dictation are never disturbed) and is remounted only when the text is replaced from
// outside while he is not typing (an Undo, a line added from a card, a newer version from another device).
// The caret is kept above the keyboard by the page's scroll view: a hidden copy of the text up to the caret, laid out
// at the input's width, gives the caret's height in the document; rules/lineRules caretScroll decides whether to move.
// Rules and their tests: rules/lineRules.ts, tests/lineRules.test.mjs.
import React, { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { AppState, PixelRatio, Platform, Text, TextInput, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { decide } from './api';
import { addVersion } from './bitWork';
import { bitDocOpen, dropBitDraft, readBitDraft, setBitPending, writeBitDraft } from './bitDraft';
import { C, F } from './theme';
import { PROSE_CAP, sourceFor, type Cut } from '@/rules/bitLines';
import { nameFromFirstLine } from '@/rules/bitWorkRules';
import * as L from '@/rules/lineRules';

const native = Platform.OS !== 'web';
const SIZE = 17;
const LINE = 26;
const MIN_H = 160;
/** How long a newer server text is ignored after our own save, while the read that carries it is on its way. */
const EXPECT_MS = 15000;
/** A save that failed while online is tried again this long after. */
const RETRY_MS = 20000;
/** The footnote line's height above the keyboard (lib/TopBar OfflineStrip: up to two 18-pt lines and its padding). */
const NOTE_H = 48;

/** Why a version is being saved. Only `leave` says so in a snack (he is no longer looking at the page); the rest
 *  are quiet: the page's footnote and its Versions row say it. */
export type SaveWhy = 'done' | 'leave' | 'pause' | 'background' | 'online';
export type BitDocState = {
  /** The keyboard is up on the document: the page keeps everything else out of the way. */ typing: boolean;
  /** Words on the page that no version holds yet (typed and not saved, or a save on its way). */ pending: boolean;
  /** One line for the page's footnote slot: why the words are not a version yet. */ note: string | null;
};
export type BitDocHandle = {
  /** The document as it stands (typed or saved). */ text: () => string;
  /** The line (counted among lines with words) the caret was last on; null when none. */ caretLine: () => number | null;
  /** Save now if the words changed; resolves when no save is on its way any more. */ save: (why: SaveWhy) => Promise<void>;
  /** The page wrote a version itself (a line from the pile, a reorder, a restore): show it, compare against it. */
  adopt: (text: string, unit?: string | null) => void;
  /** A version of ours was undone: take the server's text again when it arrives. */ undone: () => void;
  /** The scroll view changed height (the keyboard): bring the caret into view. */ follow: () => void;
  /** Put the keyboard away (Done). */ blur: () => void;
};

export type BitDocProps = {
  bit: string; cut: Cut;
  /** The version shown (its unit) and its text: what the document is compared against. */
  unit: string | null; saved: string;
  /** The text of every version of this cut: a draft that one of them already says is not his newest words. */
  said: readonly string[];
  /** A bit that has no name yet: its first save names it from its first line. */ fresh: boolean;
  autoFocus?: boolean; online: boolean;
  /** The page draws a footnote line above the keyboard (offline, or a reason): the caret is kept clear of it. */ noteOn?: boolean;
  /** A version was written: the decision indexes its Undo needs, and why it was saved. */
  onSaved: (indexes: number[], why: SaveWhy) => void;
  /** A save he asked for (Done) failed; the words stay on the phone and it is tried again. */
  onFailed?: () => void;
  onState: (s: BitDocState) => void;
  /** The page's scroll view: its ref, and where it is (kept by the page from the shell's scrollWatch). */
  scroll: { ref: React.MutableRefObject<any>; y: React.MutableRefObject<number>; h: React.MutableRefObject<number> };
  testID?: string;
};

export const BitDoc = forwardRef<BitDocHandle, BitDocProps>(function BitDoc(p, ref) {
  const ins = useSafeAreaInsets();
  const [loaded, setLoaded] = useState(false);
  // the text the input was last mounted with, and the mount it belongs to
  const [mount, setMount] = useState<{ n: number; text: string }>({ n: 0, text: p.saved });
  // the text up to the caret (native: its laid-out height is the caret's height). The text view starts with the
  // caret at the END and does not report it, so that is where this starts too.
  const [mirror, setMirror] = useState(p.saved);
  const [typing, setTyping] = useState(false);
  const [pending, setPending] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [webH, setWebH] = useState(0);
  // while typing the field is at least as tall as the page, so a tap under a short document moves the caret to its
  // end (as Notes does) instead of landing on nothing and putting the keyboard away
  const [fillH, setFillH] = useState(0);

  const input = useRef<any>(null);
  const wrap = useRef<any>(null);
  const text = useRef(p.saved);
  const base = useRef(p.saved);                        // what the words are compared against (the saved version)
  const baseUnit = useRef<string | null>(p.unit);
  const caret = useRef(p.saved.length);
  const typingRef = useRef(false);
  const pendingRef = useRef(false);
  const behind = useRef(false);                        // a newer version arrived while the keyboard was up on clean text
  const top = useRef(0);                               // the input's top in the scroll content
  const caretY = useRef(0);                            // the caret line's bottom inside the input
  const expect = useRef<{ unit: string; until: number } | null>(null);
  const named = useRef(false);
  const alive = useRef(true);
  const inFlight = useRef<Promise<void> | null>(null);
  const again = useRef<SaveWhy | null>(null);
  const lastSavedAt = useRef<number | null>(null);
  const timers = useRef<{ draft?: ReturnType<typeof setTimeout>; pause?: ReturnType<typeof setTimeout>; retry?: ReturnType<typeof setTimeout> }>({});
  const props = useRef(p);
  useEffect(() => { props.current = p; });

  const dirty = () => !L.sameDoc(text.current, base.current);
  const mark = (on: boolean) => { pendingRef.current = on; setBitPending(props.current.bit, props.current.cut, on); if (alive.current) setPending(on); };
  /** Replace the input's text from outside (never while he is typing: the caller checks). The new text view puts
   *  its caret at the end without saying so. */
  const show = useCallback((t: string) => {
    text.current = t;
    caret.current = t.length;
    setMirror(t);
    setMount((m) => ({ n: m.n + 1, text: t }));
  }, []);

  // ---- open: his draft when it holds words no version has (rules/lineRules openWith)
  useEffect(() => {
    let on = true;
    const release = bitDocOpen(p.bit, p.cut);
    void readBitDraft(p.bit, p.cut).then((d) => {
      if (!on) return;
      const o = L.openWith(base.current, d, props.current.said);
      if (o.pending) { show(o.text); mark(true); }
      else if (d) void dropBitDraft(p.bit, p.cut);
      setLoaded(true);
    });
    return () => {
      on = false;
      // the page is going: its leave-save (started by a later cleanup of this same unmount) may still be on its way,
      // and only when that has settled may a flush touch this draft
      setTimeout(() => { void (inFlight.current || Promise.resolve()).then(release, release); }, 0);
    };
  }, [p.bit, p.cut, show]);

  // ---- the server's version changed under the page
  useEffect(() => {
    if (!loaded) return;
    const e = expect.current;
    if (e) {
      // our own save: wait for the read that carries it (an older read in between must not put old words back)
      if (p.unit === e.unit || Date.now() > e.until) expect.current = null;
      else return;
    }
    if (p.unit === baseUnit.current && L.sameDoc(p.saved, base.current)) return;
    if (!dirty() && typingRef.current) {
      // the keyboard is up and he has typed nothing: the text is not swapped under his caret, and the comparison
      // stays on what he sees (so Done does not post the old words on top of the newer version); blur takes it
      behind.current = true;
      return;
    }
    const wasDirty = dirty();
    base.current = p.saved;
    baseUnit.current = p.unit;
    // his unsaved words stay (they become the newest version at the next save); a clean document follows the server
    if (!wasDirty && !L.sameDoc(text.current, p.saved)) show(p.saved);
    if (!dirty()) { mark(false); setNote(null); void dropBitDraft(p.bit, p.cut); }
  }, [p.saved, p.unit, loaded]); // eslint-disable-line react-hooks/exhaustive-deps

  // ---- saving
  const keepLocal = useCallback(async () => {
    const q = props.current;
    if (!dirty()) { await dropBitDraft(q.bit, q.cut); return true; }
    return writeBitDraft(L.bitDraft(q.bit, q.cut, baseUnit.current, text.current, new Date().toISOString()));
  }, []);

  const save = useCallback((why: SaveWhy): Promise<void> => {
    // one save at a time: a second asks again once the first has settled (set before the first await, so two calls
    // in one tick, blur then unmount, can never both post)
    if (inFlight.current) { again.current = why; return inFlight.current; }
    const run = (async () => {
      const q = props.current;
      const typed = text.current;
      const decision = L.saveDecision(base.current, typed);
      if (decision === 'none' || decision === 'empty') {
        void dropBitDraft(q.bit, q.cut);
        mark(false);
        if (alive.current) setNote(null);
        return;
      }
      // the words go to the phone before the request: a kill mid-request loses nothing
      const kept = await keepLocal();
      mark(true);
      if (decision === 'too_long') {
        if (alive.current) setNote(L.tooLongNote(typed));
        return;
      }
      if (!q.online) {
        if (alive.current) setNote(kept ? null : 'This iPhone could not keep the words');
        return;
      }
      const sent = L.docText(typed);
      try {
        const r = await addVersion(q.bit, sent, 'written on the bit page', false, sourceFor(q.cut, 'write'));
        let indexes = r.indexes;
        if (q.fresh && !named.current) {
          // a new document takes its name from its first line (rules/bitWorkRules); the version is saved either way
          named.current = true;
          try { const w = await decide({ target: 'node', action: 'edit', id: q.bit, after: { title: nameFromFirstLine(sent) }, reason: 'named from its first line' }); indexes = [...indexes, ...w.indexes]; } catch { named.current = false; }
        }
        base.current = sent;
        baseUnit.current = r.unit;
        behind.current = false;
        expect.current = { unit: r.unit, until: Date.now() + EXPECT_MS };
        lastSavedAt.current = Date.now();
        if (dirty()) void keepLocal(); else void dropBitDraft(q.bit, q.cut);
        mark(dirty());
        if (alive.current) setNote(null);
        props.current.onSaved(indexes, why);
      } catch {
        if (alive.current) {
          setNote('Not saved yet · kept on this iPhone');
          // tried again by itself while the page is open (the launch flush has it once the page is gone)
          clearTimeout(timers.current.retry);
          timers.current.retry = setTimeout(() => { if (alive.current && dirty()) void save('online'); }, RETRY_MS);
        }
        if (why === 'done') props.current.onFailed?.();
      }
    })();
    const whole = run.finally(() => {
      if (inFlight.current === whole) inFlight.current = null;
      const next = again.current;
      again.current = null;
      if (next && dirty()) return save(next);
    });
    inFlight.current = whole;
    return whole;
  }, [keepLocal]);

  const onChange = (t: string) => {
    text.current = t;
    if (native) setMirror(L.mirrorText(t, caret.current));
    clearTimeout(timers.current.draft);
    timers.current.draft = setTimeout(() => { void keepLocal().then((ok) => { if (alive.current && !ok && dirty()) setNote('This iPhone could not keep the words'); }); }, L.DRAFT_SETTLE_MS);
    clearTimeout(timers.current.pause);
    timers.current.pause = setTimeout(() => { if (L.pauseSaves(Date.now(), lastSavedAt.current)) void save('pause'); }, L.PAUSE_MS);
    // words that no version holds yet: the page keeps anything that would write over them switched off
    const d = dirty();
    if (d !== pendingRef.current && !inFlight.current) mark(d);
  };

  // the app going to the background (not `inactive`: Control Center, the app switcher and a system alert are not
  // leaving, and the words are on the phone already), and the connection coming back
  useEffect(() => {
    const sub = AppState.addEventListener('change', (s) => { if (s === 'background' && dirty()) void save('background'); });
    return () => sub.remove();
  }, [save]);
  useEffect(() => {
    if (p.online && loaded && dirty() && !typingRef.current) void save('online');
  }, [p.online, loaded, save]);
  // leaving the page (expo-router has no beforeRemove, and no blur reaches JS when a focused field is popped: the
  // save runs on unmount)
  useEffect(() => {
    const t = timers.current;
    return () => {
      alive.current = false;
      clearTimeout(t.draft);
      clearTimeout(t.pause);
      clearTimeout(t.retry);
      if (dirty()) void save('leave');
    };
  }, [save]);

  // ---- the caret stays above the keyboard (native; a browser scrolls its own caret)
  const follow = useCallback(() => {
    if (!native || !typingRef.current) return;
    const s = props.current.scroll;
    const scale = Math.min(PROSE_CAP, Math.max(1, PixelRatio.getFontScale()));
    const bottom = top.current + caretY.current;
    // the footnote line sits above the keyboard (the shell pins it `ins.bottom` up): the caret stays clear of it
    const margin = 16 + (props.current.noteOn ? ins.bottom + NOTE_H : 0);
    const y = L.caretScroll({ top: bottom - LINE * scale, bottom, offset: s.y.current, height: s.h.current, guardTop: ins.top + 52, margin });
    if (y != null) s.ref.current?.scrollTo?.({ y, animated: true });
  }, [ins.top, ins.bottom]);
  const measureTop = useCallback(() => {
    if (!native) return;
    const inner = props.current.scroll.ref.current?.getInnerViewRef?.();
    if (!inner || !wrap.current) return;
    try {
      wrap.current.measureLayout(inner, (_x: number, y: number) => { top.current = y; follow(); }, () => {});
    } catch { /* not laid out yet: the next layout measures again */ }
  }, [follow]);

  useEffect(() => { p.onState({ typing, pending, note }); }, [typing, pending, note]); // eslint-disable-line react-hooks/exhaustive-deps

  useImperativeHandle(ref, () => ({
    text: () => text.current,
    caretLine: () => L.caretLine(text.current, caret.current),
    save,
    adopt: (t: string, unit?: string | null) => {
      base.current = t;
      behind.current = false;
      if (unit) { baseUnit.current = unit; expect.current = { unit, until: Date.now() + EXPECT_MS }; }
      lastSavedAt.current = Date.now();
      show(t);
      void dropBitDraft(props.current.bit, props.current.cut);
      mark(false);
      setNote(null);
    },
    undone: () => { expect.current = null; },
    follow,
    blur: () => input.current?.blur(),
  }), [save, show, follow]);

  const onBlur = () => {
    typingRef.current = false;
    setTyping(false);
    clearTimeout(timers.current.pause);
    if (behind.current) {
      behind.current = false;
      // a newer version came in while the keyboard was up: with nothing typed, the page takes it now
      if (!dirty()) {
        const q = props.current;
        base.current = q.saved;
        baseUnit.current = q.unit;
        if (!L.sameDoc(text.current, q.saved)) show(q.saved);
        return;
      }
    }
    void save('done');
  };

  const style = { fontFamily: F.body, fontSize: SIZE, lineHeight: LINE, color: C.text, padding: 0, margin: 0 } as const;
  if (!loaded) {
    // the saved words at once, in the input's own type, while the phone is asked for a draft (a few milliseconds)
    return <Text testID={p.testID ? `${p.testID}-loading` : undefined} maxFontSizeMultiplier={PROSE_CAP} style={[style, { minHeight: MIN_H }]}>{p.saved}</Text>;
  }
  return (
    <View ref={wrap} testID={p.testID} onLayout={measureTop}>
      {native && (
        // the text up to the caret, laid out exactly as the input lays it out: its height is where the caret is
        <Text accessibilityElementsHidden importantForAccessibility="no-hide-descendants" pointerEvents="none" maxFontSizeMultiplier={PROSE_CAP}
          onLayout={(e) => { caretY.current = e.nativeEvent.layout.height; follow(); }}
          style={[style, { position: 'absolute', left: 0, right: 0, top: 0, opacity: 0 }]}>{mirror + '​'}</Text>
      )}
      <TextInput key={mount.n} ref={input} testID={p.testID ? `${p.testID}-input` : undefined} multiline scrollEnabled={false} defaultValue={mount.text}
        autoFocus={!!p.autoFocus && mount.n === 0} textAlignVertical="top" maxFontSizeMultiplier={PROSE_CAP}
        accessibilityLabel="Bit text" accessibilityHint="Saves by itself"
        placeholder={p.fresh ? 'Write the bit. The first line names it.' : 'Write the bit.'} placeholderTextColor={C.dim}
        onChangeText={onChange}
        onSelectionChange={(e) => {
          caret.current = e.nativeEvent.selection.end;
          if (native) setMirror(L.mirrorText(text.current, caret.current));
        }}
        onFocus={() => { typingRef.current = true; setFillH(props.current.scroll.h.current || 0); setTyping(true); measureTop(); }}
        onBlur={onBlur}
        onContentSizeChange={native ? undefined : (e) => setWebH(Math.ceil(e.nativeEvent.contentSize.height))}
        style={[style, { minHeight: native && typing ? Math.max(MIN_H, fillH) : MIN_H }, !native && webH ? { height: Math.max(MIN_H, webH) } : null, !native ? ({ outlineStyle: 'none' } as any) : null]} />
    </View>
  );
});
