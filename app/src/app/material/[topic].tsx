// Material (SPEC 2026-11-06 §12.4): a sheet over the script, 3c's pile sheet: the medium detent first (the script
// stays in sight and live behind it: nonmodal there), large on drag, the grabber, no Close (a swipe dismisses). A plain
// filter field, then his jots, what he kept, what was found: each line its own row with a 44-pt +. One tap puts that
// one line after the line he is on, word for word, with its source ("from Free employee v3"); "Added" with Undo; his
// place stays. A line already in the rant reads "in the rant". Reads the topic page (cached: offline it is the last read).
import React, { useState } from 'react';
import { Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import { useLocalSearchParams } from 'expo-router';
import { useApi } from '@/lib/api';
import { scripts, useScript } from '@/lib/scriptStore';
import { haptics } from '@/lib/haptics';
import { Icon } from '@/lib/Icon';
import { C, F, linkColor } from '@/lib/theme';
import { T, useUI } from '@/lib/ui';
import { PROSE_CAP, ROW_CAP, linesOf } from '@/rules/bitLines';
import { cardHead, materialRows, sourceLabel, type Mat, type TopicPage } from '@/rules/rantRules';
import { inRant, pullLine, removeKeys } from '@/rules/scriptRules';

const enc = encodeURIComponent;

export default function MaterialSheet() {
  const p = useLocalSearchParams<{ topic: string }>();
  const tid = decodeURIComponent(String(p.topic || ''));
  const ui = useUI();
  const st = useScript(tid);
  const pg = useApi<TopicPage>(tid ? `/api/topics/${enc(tid)}` : null);
  const [q, setQ] = useState('');
  const rows = materialRows(pg.data);
  const mats: Mat[] = [...rows.kept, ...rows.found];
  const want = q.trim().toLowerCase();
  const shown = want ? mats.filter((m) => `${cardHead(m)} ${m.words}`.toLowerCase().includes(want)) : mats;
  const lines = st?.lines || [];

  const pull = (m: Mat, text: string) => {
    const cur = scripts.get(tid);
    if (!cur) return;
    const r = pullLine(cur.lines, cur.at, text, m.kind === 'jot' ? null : m.ref, sourceLabel(m));
    const key = r.lines[r.focus].key;
    scripts.setLines(tid, r.lines, r.focus);
    haptics.success();
    // Undo takes out this line only; his place stays where it was
    ui.snack('Added', () => { const now = scripts.get(tid); if (now) scripts.setLines(tid, removeKeys(now.lines, [key]), cur.at); });
  };

  return (
    <ScrollView testID="material" style={{ flex: 1, backgroundColor: C.surface }} contentContainerStyle={{ padding: 16, paddingTop: 20, gap: 10 }} keyboardShouldPersistTaps="handled">
      <TextInput testID="material-filter" value={q} onChangeText={setQ} placeholder="Search your material" placeholderTextColor={C.dim} accessibilityLabel="Search your material"
        autoCapitalize="none" maxFontSizeMultiplier={ROW_CAP} style={{ minHeight: 44, borderRadius: 10, paddingHorizontal: 12, backgroundColor: C.card, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      {!pg.data && !pg.rawError && <T v="mut">Reading your material…</T>}
      {!st && <T v="mut" testID="material-closed">Open the rant to add lines to it.</T>}
      {pg.data && !shown.length && <T v="mut" testID="material-empty">{want ? `Nothing of yours says “${q.trim()}”` : 'Nothing kept or found on this topic yet.'}</T>}
      {shown.map((m) => (
        <View key={m.ref} style={{ gap: 2 }}>
          <Text accessibilityRole="header" maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 13, color: C.mut2, marginTop: 8 }}>
            {`${cardHead(m)}${rows.kept.includes(m) ? ' · kept' : ''}`}
          </Text>
          {linesOf(m.words).map((text, i) => {
            const there = inRant(lines, text);
            return (
              <View key={i} style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44, borderTopWidth: i ? 1 : 0, borderTopColor: C.hairline }}>
                <Text maxFontSizeMultiplier={PROSE_CAP} style={{ flex: 1, fontFamily: F.body, fontSize: 16, lineHeight: 22, color: C.text, paddingVertical: 6 }}>{text}</Text>
                {there ? (
                  <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>in the rant</Text>
                ) : (
                  <Pressable testID={`material-add-${m.ref}-${i}`} accessibilityRole="button" accessibilityLabel={`Add this line to the rant: ${text}`} disabled={!st}
                    onPress={() => pull(m, text)} style={({ pressed }) => ({ width: 44, height: 44, alignItems: 'center', justifyContent: 'center', opacity: pressed ? 0.6 : 1 })}>
                    <Icon name="plus" size={20} color={linkColor()} sw={2.4} />
                  </Pressable>
                )}
              </View>
            );
          })}
        </View>
      ))}
    </ScrollView>
  );
}
