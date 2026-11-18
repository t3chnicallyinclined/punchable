// His marks on a page, listed under it (docs/design/SPEC-2026-11-17-one-notebook.md screen 3). The page is one text
// field, so a note cannot be drawn between two of its paragraphs: each mark is a row here instead, in the page's
// order: how he asked (Funnier, Shorter, 3 options, Note), the words it is on, and his note ("Your note: …"). A tap
// opens the one sheet a mark has: its note, and Remove. This replaces the marks tray. A mark whose words he has since
// rewritten is shown as such and is never sent (rules/markRules placed).
import React, { useState } from 'react';
import { Pressable, Text, TextInput, View } from 'react-native';
import { C, F, linkColor } from './theme';
import { Btn, Sheet, SheetHead, T } from './ui';
import { ROW_CAP } from '@/rules/bitLines';
import { MAX_NOTE, placed, type Mark } from '@/rules/markRules';
import { askWord } from '@/rules/pageRules';
import { brief } from '@/rules/draftRules';

export function MarksList({ text, marks, editing, onEdit, onSave, onRemove, testID }: {
  /** The page's text now (asked for when the list draws: the document owns it). */ text: () => string; marks: readonly Mark[];
  /** The mark whose sheet is open (the bar's Note opens it too). */ editing: string | null;
  onEdit: (id: string | null) => void;
  onSave: (id: string, note: string) => void;
  onRemove: (id: string) => void;
  testID?: string;
}) {
  const rows = placed(text(), [...marks]);
  const open = editing ? marks.find((m) => m.id === editing) || null : null;
  return (
    <View testID={testID} style={{ gap: 2 }}>
      {rows.map(({ mark, gone }) => {
        const word = askWord(mark);
        const note = mark.note.trim();
        return (
          <Pressable key={mark.id} testID={testID ? `${testID}-${mark.id}` : undefined} accessibilityRole="button"
            accessibilityLabel={`${word} on “${brief(mark.text, 60)}”${note ? `. Your note: ${note}` : ''}${gone ? '. These words changed; it will not be sent' : ''}`}
            accessibilityHint="Opens its note and Remove" onPress={() => onEdit(mark.id)}
            style={({ pressed }) => ({ minHeight: 44, flexDirection: 'row', alignItems: 'flex-start', gap: 10, paddingVertical: 8, backgroundColor: pressed ? C.card2 : 'transparent' })}>
            <View style={{ width: 8, height: 8, borderRadius: 4, marginTop: 6, backgroundColor: gone ? C.borderStrong : C.amber }} />
            <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
              <Text numberOfLines={2} maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 20, color: gone ? C.dim : C.mut2 }}>
                <Text style={{ fontFamily: F.semi, color: gone ? C.dim : C.text }}>{word}</Text>
                {`  “${brief(mark.text, 90)}”`}
              </Text>
              {!!note && <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 20, color: C.mut2 }}>Your note: “{note}”</Text>}
              {gone && <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.dim }}>You changed these words. This mark will not be sent.</Text>}
            </View>
          </Pressable>
        );
      })}
      <Sheet visible={!!open} onClose={() => onEdit(null)} label="A mark" testID={testID ? `${testID}-sheet` : undefined}>
        {open && <NoteForm key={open.id} mark={open} onCancel={() => onEdit(null)} onSave={(n) => onSave(open.id, n)} onRemove={() => onRemove(open.id)} testID={testID} />}
      </Sheet>
    </View>
  );
}

/** One mark's sheet: Cancel leading, Done trailing (HIG/sheets), the words it is on, the note, Remove. */
function NoteForm({ mark, onCancel, onSave, onRemove, testID }: { mark: Mark; onCancel: () => void; onSave: (note: string) => void; onRemove: () => void; testID?: string }) {
  const [v, setV] = useState(mark.note);
  const word = askWord(mark);
  return (
    <View style={{ gap: 10 }}>
      <SheetHead title={word === 'Note' ? 'Note' : `${word} · note`} onCancel={onCancel} onDone={() => onSave(v)} testID={testID ? `${testID}-note` : undefined} />
      <T v="mut" numberOfLines={3}>On “{brief(mark.text, 160)}”</T>
      <TextInput testID={testID ? `${testID}-note-input` : undefined} value={v} onChangeText={setV} multiline autoFocus maxLength={MAX_NOTE} accessibilityLabel="Your note on these words"
        placeholder={word === 'Note' ? 'What should change here?' : 'Anything to add? (optional)'} placeholderTextColor={C.dim}
        style={{ minHeight: 88, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, paddingVertical: 10, color: C.text, fontFamily: F.body, fontSize: 17, lineHeight: 24 }} />
      <Btn testID={testID ? `${testID}-remove` : undefined} v="ghost" label="Remove this mark" textStyle={{ color: linkColor() }} onPress={onRemove} />
    </View>
  );
}
