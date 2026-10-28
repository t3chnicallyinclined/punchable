// A weak transition between two bits of a set list: an optional AI bridge draft (cached, or queued for his home
// worker) and his own line, saved as his segue (a decision line, undoable). Used by the set list (/list) and the old
// builder (/builder).
import React, { useState } from 'react';
import { Text, TextInput, View } from 'react-native';
import { decide, undo } from './api';
import { Bridge, creative } from './creative';
import { C, F } from './theme';
import { Btn, Sheet, SheetHead, T, s, useUI } from './ui';
import { WorkerLine } from './worker';

export type Gap = { from: string; to: string; words: string };

/** A weak transition: an optional AI bridge draft (cached, or queued for the writer on the server), and his own line. */
export function BridgeSheet({ gap, boardId, titleOf, onClose }: { gap: Gap | null; boardId: string; titleOf: (j: string) => string; onClose: () => void }) {
  const ui = useUI();
  const [state, setState] = useState<Bridge | null>(null);
  const [mine, setMine] = useState('');
  const [busy, setBusy] = useState(false);
  if (!gap) return <Sheet visible={false} onClose={onClose} label="Bridge">{null}</Sheet>;
  const ask = () => {
    setBusy(true);
    creative.bridge(boardId, gap.from, gap.to).then(setState)
      .catch((e) => setState({ from: gap.from, to: gap.to, draft: true, label: '', status: 'provider_error', text: e.message }))
      .finally(() => setBusy(false));
  };
  const save = async (text: string) => {
    onClose();
    try {
      const w = await decide({ target: 'edge', action: 'add', after: { src: gap.from, dst: gap.to, kind: 'segue', evidence: [], bridge: text }, reason: `bridge written on board: ${text.slice(0, 160)}` });
      ui.snack('Saved as your segue', async () => { await undo(w.indexes); });
    } catch (e: any) {
      ui.snack(`Couldn't save: ${e.message}`);
    }
  };
  return (
    <Sheet visible onClose={onClose} label="Bridge" testID="bridge-sheet">
      <SheetHead title="Bridge" onCancel={onClose} testID="bridge" />
      <T v="eb">Between</T>
      <T v="h2">{titleOf(gap.from)} → {titleOf(gap.to)}</T>
      <T v="mut" style={{ marginVertical: 6 }}>{gap.words}</T>
      {!state && !busy && <Btn testID="bridge-ask" small v="add" label="Get a bridge idea (draft)" onPress={ask} style={{ alignSelf: 'flex-start' }} />}
      {busy && <T v="mut">Asking for a bridge…</T>}
      {state?.status === 'queued' && <View testID="bridge-queued" style={[s.inner, { gap: 4 }]}><T v="semi">Queued</T><T v="mut">The writer drafts bridges. It shows up here when it’s done.</T><WorkerLine what="The bridge draft" /></View>}
      {state?.status === 'no_grounding' && <T v="mut" testID="bridge-none">Nothing links these two to build a bridge from. Write your own below, or move one.</T>}
      {state?.status === 'provider_error' && <T v="mut">The AI didn’t answer. Write your own below.</T>}
      {state?.status === 'ok' && state.text && (
        <View testID="bridge-draft" style={[s.draft, { gap: 8, marginVertical: 6 }]}>
          <View style={{ flexDirection: 'row', gap: 6, alignItems: 'center' }}>
            <View style={{ paddingHorizontal: 6, minHeight: 20, borderRadius: 4, backgroundColor: C.addBg, justifyContent: 'center' }}><Text style={{ fontFamily: F.g7, fontSize: 11, color: C.amber }}>Draft</Text></View>
            <T v="small" style={{ flex: 1 }}>not your words yet</T>
          </View>
          <Text style={{ fontFamily: F.body, fontSize: 16, lineHeight: 23, color: C.amber }}>{state.text}</Text>
          <View style={{ flexDirection: 'row', gap: 8 }}>
            <Btn testID="bridge-use" small label="Use as mine" onPress={() => save(state.text!)} />
            <Btn small v="ghost" label="Edit" onPress={() => setMine(state.text!)} />
          </View>
        </View>
      )}
      <T v="small" style={{ marginTop: 8 }}>Your own bridge</T>
      <TextInput testID="bridge-mine" value={mine} onChangeText={setMine} multiline placeholder="Speaking of…" placeholderTextColor={C.dim}
        style={{ minHeight: 72, borderWidth: 1, borderColor: C.border, borderRadius: 8, padding: 12, color: C.text, fontFamily: F.body, fontSize: 16, textAlignVertical: 'top' }} />
      <Btn testID="bridge-save" label="Save my bridge" v="primary" disabled={!mine.trim()} onPress={() => save(mine.trim())} style={{ marginTop: 8 }} />
    </Sheet>
  );
}

