// His bullet notes under one bit of a set list (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.3, Slice 4): the
// notes he reads on stage. Reading: plain bullets that wrap; a tap on one puts the cursor in it. Writing: one single-
// line field per note (iOS reports Backspace in an empty SINGLE-line field, never in a multiline one:
// RCTBackedTextInputDelegateAdapter.mm keyboardInputShouldDelete), Return adds a note under the one he is in, Return
// on an empty one puts the keyboard away, Backspace in an empty one removes it, "Add a note" adds one at the end.
// While he has written none, the bit's own words are shown muted under "From the bit" and are never editable or
// saved: his first note replaces them (rules/setNoteRules.ts, the rule its --red arm pins).
//
// Saving: one POST /api/boards/<id>/notes with the bit's whole list when the cursor leaves this bit's notes, when the
// app leaves the foreground mid-typing, and when the screen goes; never per keystroke or per Return. Adding is quiet;
// a note that went or changed gets a snack with Undo; every save hands its Undo to the screen (`onUndoable`) for the
// menu's untimed "Undo". A save that fails keeps his words: "Not saved · Try again" under the bit, and in `unsent`
// for as long as the app runs, so the row is still there when he comes back to the set. A change that arrives from
// another device while he is typing here is not taken; his next save wins. One file for the phone and the web.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AppState, Pressable, Text, TextInput, View } from 'react-native';
import { ApiError, post } from './api';
import * as N from '@/rules/setNoteRules';
import { C, F, linkColor } from './theme';
import { Icon, useUI } from './ui';

type Saved = { notes?: string[]; was?: string[] };

/** Lists a save could not deliver, by `<board>|<joke>`. In memory only: his words never go to the phone's storage here. */
const unsent = new Map<string, string[]>();

export function BulletNotes({ boardId, jokeId, name, step, canEdit, why, onSaved, onUndoable, indent = 0, testID }: {
  boardId: string; jokeId: string; name: string; step: N.NotedStep;
  /** False while he cannot write (offline): the notes read, the fields do not open, and `why` ("offline") says so. */
  canEdit: boolean; why?: string;
  /** His notes for this bit as the server now has them (the list page re-reads the build; the stage sheet is told). */
  onSaved: (notes: string[]) => void;
  /** A save landed and can be taken back: what changed, in the snack's words, and the call that undoes it; null
   *  once that Undo has run (from the snack or the menu), so the menu does not offer it twice. */
  onUndoable?: (what: string, undo: (() => Promise<void>) | null) => void;
  indent?: number; testID?: string;
}) {
  const ui = useUI();
  const slot = `${boardId}|${jokeId}`;
  const [drafts, setDraftsState] = useState<N.Draft[]>(() => {
    const kept = unsent.get(slot);
    return kept ? N.editorNotes({ notes: kept, notes_from: 'his' }) : N.editorNotes(step);
  });
  const [editing, setEditing] = useState(false);
  const [failed, setFailed] = useState(() => unsent.has(slot));
  const draftsRef = useRef(drafts);
  const savedRef = useRef<string[]>(N.hisNotes(step));
  const inputs = useRef(new Map<string, React.ComponentRef<typeof TextInput>>());
  const focused = useRef<string | null>(null);
  const busy = useRef(false);
  const again = useRef(false);
  const alive = useRef(true);
  const onSavedRef = useRef(onSaved);
  const onUndoableRef = useRef(onUndoable);
  const saveRef = useRef<() => Promise<void>>(async () => {});
  /** The bullet that takes the cursor once it is on screen (set by an edit, used by the effect below). */
  const wantFocus = useRef<string | null>(null);
  const setDrafts = useCallback((d: N.Draft[]) => { draftsRef.current = d; setDraftsState(d); }, []);

  // the server's list changed under us (another device, a pull to refresh, Undo elsewhere): take it, unless he is in
  // the middle of writing here or a save of his is on its way or waiting to be retried
  const served = N.hisNotes(step);
  const servedKey = JSON.stringify(served);
  useEffect(() => {
    if (editing || busy.current || failed) return;
    if (N.sameNotes(served, savedRef.current) && N.sameNotes(served, N.cleanNotes(draftsRef.current.map((d) => d.text)))) return;
    savedRef.current = served;
    setDrafts(N.editorNotes(step));
  }, [servedKey]); // eslint-disable-line react-hooks/exhaustive-deps

  /** He is done with this bit's notes: reading rows again, the blank bullets gone, and what changed is saved. */
  const leave = () => {
    focused.current = null;
    wantFocus.current = null;
    setEditing(false);
    setDrafts(N.onLeave(draftsRef.current));
    void saveRef.current();
  };

  // a plain function (it reads refs and props only); the effect below keeps the newest one in `saveRef`
  const save = async () => {
    if (busy.current) { again.current = true; return; }
    const call = N.planSave(boardId, jokeId, savedRef.current, draftsRef.current);
    if (!call) { unsent.delete(slot); if (alive.current) setFailed(false); return; }
    busy.current = true;
    const before = savedRef.current;
    try {
      const res = await post<Saved>(call.path, call.body);
      const now = Array.isArray(res?.notes) ? res.notes : (call.body.notes as string[]);
      savedRef.current = now;
      unsent.delete(slot);
      onSavedRef.current(now);
      if (alive.current) setFailed(false);
      const back = N.undoSave(boardId, jokeId, { notes: now, was: Array.isArray(res?.was) ? res.was : before });
      const undo = back ? async () => {
        const r = await post<Saved>(back.path, back.body);
        const notes = Array.isArray(r?.notes) ? r.notes : (back.body.notes as string[]);
        savedRef.current = notes;
        unsent.delete(slot);
        onSavedRef.current(notes);
        onUndoableRef.current?.('', null);
        if (!alive.current) return;
        // he may be in one of this bit's fields: it is about to be replaced, and no blur would say so
        focused.current = null;
        wantFocus.current = null;
        setEditing(false);
        setFailed(false);
        setDrafts(N.editorNotes({ notes, notes_from: 'his' }));
      } : undefined;
      if (undo) onUndoableRef.current?.(N.saveWords(before, now), undo);
      const msg = N.saveSnack(before, now);
      if (msg) ui.snack(msg, undo);
    } catch (e: any) {
      unsent.set(slot, call.body.notes as string[]);
      if (alive.current) setFailed(true);
      // the server's own words when it answered (an archived list, court material); else nothing reached it. When the
      // screen is already gone there is no row to retry from, so the words say where the retry is.
      ui.snack(e instanceof ApiError ? `Couldn’t save the notes: ${e.message}`
        : alive.current ? 'Can’t reach Punchable. Your notes are not saved yet.'
          : 'Can’t reach Punchable. The notes you just typed are not saved. Open the set to try again.');
    } finally {
      busy.current = false;
      if (again.current) { again.current = false; void saveRef.current(); }
    }
  };

  useEffect(() => { onSavedRef.current = onSaved; onUndoableRef.current = onUndoable; saveRef.current = save; });
  // leaving the screen with something unsaved: one last save (the snack is the app's, so a failure is still said)
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; void saveRef.current(); };
  }, []);
  // the app leaves the foreground while he is typing (a call, the Home screen): save what is there, in case iOS ends
  // the app before he comes back
  useEffect(() => {
    if (!editing) return;
    const sub = AppState.addEventListener('change', (s) => { if (s !== 'active') void saveRef.current(); });
    return () => sub.remove();
  }, [editing]);

  // the cursor goes where the last edit said, once that field is on screen
  useEffect(() => {
    const key = wantFocus.current;
    if (!key || !editing) return;
    if (!drafts.some((d) => d.key === key)) { wantFocus.current = null; return; }
    const field = inputs.current.get(key);
    if (field) { wantFocus.current = null; field.focus(); }
  }, [drafts, editing]);

  const apply = (e: N.Edit) => {
    const removed = e.drafts.length < draftsRef.current.length;
    if (e.focus) wantFocus.current = e.focus;
    setDrafts(e.drafts);
    // Return on an empty bullet: the keyboard goes, and the blur below does the leaving
    if (e.done) { inputs.current.get(focused.current || '')?.blur(); return; }
    // the field he was in went with its bullet and no other takes the cursor: no blur will come, so leave here
    if (removed && !e.focus) leave();
  };
  const open = (key: string | null) => {
    if (!canEdit) return;
    if (key) wantFocus.current = key;
    else apply(N.addNote(draftsRef.current));
    setEditing(true);
  };
  const onBlur = (key: string) => {
    if (focused.current === key) focused.current = null;
    // the cursor may be on its way to another note of this bit (Return, a tap): wait a beat before calling it a leave
    setTimeout(() => { if (alive.current && focused.current === null) leave(); }, 80);
  };

  const derived = N.derivedNotes(step);
  // the bit's own words stay in sight while he writes his first note; the build that follows his save carries his
  const showDerived = derived.length > 0 && (editing || !drafts.length);
  const tint = canEdit ? linkColor() : C.dim;
  const bullet = (color: string) => <Text accessible={false} maxFontSizeMultiplier={2} style={{ width: 14, fontFamily: F.body, fontSize: 16, lineHeight: 22, color }}>•</Text>;
  const derivedLines = (
    <>
      <Text accessible={false} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.medium, fontSize: 12, lineHeight: 16, color: C.dim }}>From the bit</Text>
      {derived.map((t, j) => (
        <View key={j} style={{ flexDirection: 'row', gap: 6 }}>
          {bullet(C.dim)}
          <Text numberOfLines={2} maxFontSizeMultiplier={2} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 15, lineHeight: 22, color: C.mut2 }}>{t}</Text>
        </View>
      ))}
    </>
  );

  return (
    <View testID={testID} style={{ paddingLeft: indent, paddingRight: 8, paddingBottom: 4 }}>
      {showDerived && (editing ? (
        <View testID={testID ? `${testID}-derived` : undefined} accessible accessibilityLabel={`From the bit: ${derived.join(' ')}`} style={{ gap: 2, paddingVertical: 4 }}>{derivedLines}</View>
      ) : (
        <Pressable testID={testID ? `${testID}-derived` : undefined} disabled={!canEdit} accessibilityRole="button"
          accessibilityLabel={`From the bit: ${derived.join(' ')}`} accessibilityHint={canEdit ? 'Write your own note; it replaces these lines' : `You are ${why}`}
          onPress={() => open(null)} style={({ pressed }) => ({ gap: 2, paddingVertical: 4, opacity: pressed ? 0.6 : 1 })}>
          {derivedLines}
        </Pressable>
      ))}
      {!editing && drafts.map((d, j) => (
        <Pressable key={d.key} testID={testID ? `${testID}-note-${j}` : undefined} disabled={!canEdit} accessibilityRole="button"
          accessibilityLabel={`${N.noteLabel(j, drafts.length, name)}: ${d.text}`} accessibilityHint={canEdit ? 'Edit this note' : `You are ${why}`}
          onPress={() => open(d.key)} style={({ pressed }) => ({ flexDirection: 'row', gap: 6, minHeight: 30, paddingVertical: 4, opacity: pressed ? 0.6 : 1 })}>
          {bullet(C.mut2)}
          <Text maxFontSizeMultiplier={2} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 16, lineHeight: 22, color: C.text }}>{d.text}</Text>
        </Pressable>
      ))}
      {editing && drafts.map((d, j) => (
        <View key={d.key} style={{ flexDirection: 'row', alignItems: 'center', gap: 6, minHeight: 44 }}>
          {bullet(C.mut2)}
          <TextInput ref={(r) => { if (r) inputs.current.set(d.key, r); else inputs.current.delete(d.key); }}
            testID={testID ? `${testID}-input-${j}` : undefined} value={d.text} maxLength={N.MAX_NOTE_CHARS} maxFontSizeMultiplier={2}
            onChangeText={(t) => setDrafts(N.onType(draftsRef.current, d.key, t))}
            onFocus={() => { focused.current = d.key; }} onBlur={() => onBlur(d.key)}
            // Return: a new note under this one, and the keyboard stays (`blurOnSubmit` is what the web reads); Return
            // on an empty note ends the writing. The plain Return key: it makes a line, as in Notes.
            submitBehavior="submit" blurOnSubmit={false}
            onSubmitEditing={() => apply(N.onReturn(draftsRef.current, d.key))}
            onKeyPress={(e) => { if (e.nativeEvent.key === 'Backspace' && d.text === '') apply(N.onBackspace(draftsRef.current, d.key)); }}
            placeholder="A note" placeholderTextColor={C.dim} accessibilityLabel={N.noteLabel(j, drafts.length, name)}
            accessibilityHint="Return adds a note. Return on an empty note finishes. Delete on an empty note removes it."
            style={{ flex: 1, minWidth: 0, minHeight: 44, paddingVertical: 0, fontFamily: F.body, fontSize: 16, color: C.text }} />
        </View>
      ))}
      {failed && !editing && (
        <Pressable testID={testID ? `${testID}-retry` : undefined} accessibilityRole="button" accessibilityLabel="Notes not saved. Try again" onPress={() => void saveRef.current()}
          style={({ pressed }) => ({ minHeight: 44, justifyContent: 'center', opacity: pressed ? 0.6 : 1 })}>
          <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 13, color: C.gapText }}>Not saved · Try again</Text>
        </Pressable>
      )}
      {N.canAddNote(drafts) && (
        <Pressable testID={testID ? `${testID}-add` : undefined} disabled={!canEdit} accessibilityRole="button" accessibilityLabel={`Add a note for ${name}`}
          accessibilityHint={canEdit || !why ? undefined : `You are ${why}`} accessibilityState={{ disabled: !canEdit }} onPress={() => open(null)}
          style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 6, minHeight: 44, opacity: pressed ? 0.6 : 1 })}>
          {/* light-mode amber is never text (DESIGN.md): the link colour, as every other text button */}
          <Icon name="plus" size={14} color={tint} sw={2.4} />
          <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.medium, fontSize: 14, color: tint }}>{canEdit || !why ? 'Add a note' : `Add a note · ${why}`}</Text>
        </Pressable>
      )}
    </View>
  );
}
