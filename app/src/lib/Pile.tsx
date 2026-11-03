// The pile, inline under its row (SPEC 2026-11-03 §3.4; the Slice 1 subset, §14): every scrap gathered for this bit
// but not placed in its text. Today's data: the notes attached to the bit (the merge store's attach records) and the
// voice memos on this phone that are on it. A scrap row: a 44-pt leading glyph for the kind (for a memo the glyph IS
// the play button), the words at two lines, a trailing 44-pt + ("Add to the bit", after the line he is on). A scrap
// already in the bit reads "in the bit" instead of the + (no placement record until Slice 3). Empty: one row, "Gather
// from your notes ›" (Loose material until Gather lands in Slice 5). The five newest inline, then "N more".
import React, { useState } from 'react';
import { PixelRatio, Pressable, Text, View } from 'react-native';
import { router } from 'expo-router';
import { PILE_INLINE, ROW_CAP, inBit, type Scrap } from '@/rules/bitLines';
import { statusWords } from '@/rules/memoRules';
import { Icon } from './Icon';
import { playLocal, useLocalPlaying } from './localPlay';
import { uriOf } from './recordings';
import type { LocalRec } from './recordings';
import { isRecording } from './audioBusy';
import { C, F, linkColor } from './theme';
import { AIBadge, IconBtn, useUI } from './ui';

function PlayGlyph({ rec }: { rec: LocalRec }) {
  const ui = useUI();
  const p = useLocalPlaying();
  const key = `rec:${rec.id}`;
  const on = p.key === key && p.playing;
  const play = () => {
    if (isRecording()) return ui.snack('Stop the recording first.');
    playLocal(uriOf(rec), key).catch((e) => ui.snack(e?.message || 'Couldn’t play it'));
  };
  return (
    <IconBtn testID={`pile-play-${rec.id}`} label={on ? 'Pause' : 'Play memo'} onPress={play}>
      <Icon name={on ? 'pause' : 'play'} size={20} color={linkColor()} />
    </IconBtn>
  );
}

function ScrapRow({ s, rec, lines, onAdd, testID }: { s: Scrap; rec?: LocalRec; lines: string[]; onAdd: (s: Scrap) => void; testID?: string }) {
  const placed = inBit(lines, s.words);
  const words = s.words || (rec ? statusWords(rec) : '');
  const canAdd = !!s.words && !placed;
  return (
    <View testID={testID} style={{ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 56, paddingVertical: 4 }}>
      {rec ? <PlayGlyph rec={rec} /> : (
        <View style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }} accessibilityLabel={s.kind === 'doc' ? 'Doc' : 'Jot'}>
          <Icon name={s.kind === 'doc' ? 'desk' : 'notes'} size={20} color={C.mut2} />
        </View>
      )}
      <View accessible accessibilityLabel={`${s.kind === 'memo' ? 'Memo' : s.kind === 'doc' ? 'Doc' : 'Jot'}${s.ai ? ', filed by the AI' : ''}: ${words}${placed ? '. In the bit' : ''}`} style={{ flex: 1, minWidth: 0, flexDirection: 'row', alignItems: 'center', gap: 6 }}>
        {s.ai && <AIBadge testID={testID ? `${testID}-ai` : undefined} />}
        <Text maxFontSizeMultiplier={ROW_CAP} numberOfLines={2} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 15, lineHeight: 21, color: s.words ? C.text2 : C.mut2 }}>{words}</Text>
      </View>
      {placed ? (
        <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>in the bit</Text>
      ) : canAdd ? (
        <Pressable testID={testID ? `${testID}-add` : undefined} accessibilityRole="button" accessibilityLabel="Add to the bit" accessibilityHint="Puts it after the line you are on" onPress={() => onAdd(s)}
          style={({ pressed }) => ({ width: 44, height: 44, borderRadius: 12, borderWidth: 1, borderColor: C.border, backgroundColor: pressed ? C.card2 : C.card, alignItems: 'center', justifyContent: 'center' })}>
          <Icon name="plus" size={18} color={linkColor()} sw={2.4} />
        </Pressable>
      ) : null}
    </View>
  );
}

export function Pile({ scraps, recs, lines, onAdd, testID }: { scraps: Scrap[]; recs: LocalRec[]; lines: string[]; onAdd: (s: Scrap) => void; testID?: string }) {
  const [all, setAll] = useState(false);
  const fs = PixelRatio.getFontScale();
  const byId = new Map(recs.map((r) => [r.id, r]));
  if (!scraps.length) {
    return (
      <Pressable testID={testID ? `${testID}-gather` : undefined} accessibilityRole="button" onPress={() => router.push('/inbox' as any)}
        style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 48, paddingVertical: 6, borderBottomWidth: 1, borderBottomColor: C.hairline, opacity: pressed ? 0.7 : 1 })}>
        <Text maxFontSizeMultiplier={ROW_CAP} style={{ flex: 1, fontFamily: F.semi, fontSize: 17, lineHeight: 22, color: linkColor() }}>Gather from your notes</Text>
        <Icon name="chevron" size={Math.round(16 * Math.min(ROW_CAP, fs))} color={C.dim} />
      </Pressable>
    );
  }
  const shown = all ? scraps : scraps.slice(0, PILE_INLINE);
  const more = scraps.length - shown.length;
  return (
    <View testID={testID} style={{ paddingVertical: 4, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
      {shown.map((s) => <ScrapRow key={s.id} s={s} rec={s.recId ? byId.get(s.recId) : undefined} lines={lines} onAdd={onAdd} testID={testID ? `${testID}-${s.id}` : undefined} />)}
      {more > 0 && (
        <Pressable testID={testID ? `${testID}-more` : undefined} accessibilityRole="button" accessibilityState={{ expanded: false }} onPress={() => setAll(true)} style={({ pressed }) => ({ minHeight: 44, justifyContent: 'center', paddingLeft: 54, opacity: pressed ? 0.7 : 1 })}>
          <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 15, lineHeight: 20, color: linkColor() }}>{more} more</Text>
        </Pressable>
      )}
    </View>
  );
}
