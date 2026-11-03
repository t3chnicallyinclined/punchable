// On tape (SPEC 2026-11-03 §3.7): a bit's tape history, pushed from the On tape row. Today's "Elsewhere" block in
// full: the performed rows (each a deep link into the set at that second), the taped moments that sound like it but
// are not linked ("link it": a performed_as edge, undoable), and the bits that sound like it ("merge it in", an
// action sheet on the phone). One read: /api/jokes/<id>/similar (api-dev/search_api.py). No map. Plus, from Film it
// (SPEC 2026-11-03 §2.7), the FILMED rows: the takes he kept on this phone, read from the recordings index
// ("Filmed · Nov 3 · 0:48 · 112 words"; the play glyph opens the take; Delete the words is the row's own).
import React, { useMemo } from 'react';
import { Platform, Pressable, Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { decide, undo, useApi } from '@/lib/api';
import { mergeInto, undoMerge } from '@/lib/bitWork';
import { MomentRow } from '@/lib/MomentRow';
import { RecRows } from '@/lib/RecRows';
import { useRecordings } from '@/lib/recordings';
import { closeness, oneLine, type SimilarResult } from '@/rules/searchRows';
import { countWord, killedCount, killedWord } from '@/rules/bitLines';
import { shortNameOf } from '@/lib/say';
import { C, F } from '@/lib/theme';
import { haptics } from '@/lib/haptics';
import { actionSheet } from '@/lib/actionSheet';
import { Btn, ErrorCard, Skeleton, T, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const enc = encodeURIComponent;
const native = Platform.OS !== 'web';

export default function TapePage() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const d = useApi<any>(`/api/jokes/${enc(id)}`);
  const sim = useApi<SimilarResult>(`/api/jokes/${enc(id)}/similar?limit=8`);
  const J = d.data?.joke;
  const shortName = shortNameOf(J?.title || '', J?.short_name);
  const reload = () => { d.reload(); sim.reload(); };
  const recs = useRecordings();
  const takes = useMemo(() => recs.filter((r) => r.kind === 'film' && r.bit?.id === id), [recs, id]);

  const mergeLike = (b: { id: string; title: string; short_name?: string | null }) => {
    const name = shortNameOf(b.title, b.short_name);
    const go = async () => {
      try {
        const r = await mergeInto(id, [b.id], `merged from On tape on ${shortName}`);
        haptics.success();
        reload();
        ui.snack(`${name} is now part of this bit`, async () => { await undoMerge(r.merge_id); reload(); });
      } catch (e: any) {
        ui.snack(`Couldn’t merge: ${e.message}`);
      }
    };
    if (native) actionSheet({ title: `Merge “${name}” into ${shortName}?`, message: 'Its versions and tapes move under this bit. You can undo it.', options: [{ label: 'Merge', onPress: go }, { label: 'Cancel', cancel: true }] });
    else go();
  };
  const linkMoment = async (pid: string) => {
    try {
      const w = await decide({ target: 'edge', action: 'add', after: { src: pid, dst: id, kind: 'performed_as' }, reason: 'linked from On tape' });
      haptics.success();
      reload();
      ui.snack('Linked: that tape is this bit', async () => { await undo(w.indexes); reload(); });
    } catch (e: any) {
      ui.snack(`Couldn’t link it: ${e.message}`);
    }
  };

  if (sim.rawError && !sim.data) return <Screen title="On tape" back><ErrorCard error={String(sim.rawError)} onRetry={reload} /></Screen>;
  if (!sim.data) return <Screen title="On tape" back><Skeleton lines={5} height={18} testID="tape-skeleton" /></Screen>;
  const s = sim.data;
  const nights = s.performed.length;
  const killed = killedCount(s.performed as { rating?: string | null }[]);
  return (
    <Screen title="On tape" back testID="tapepage" maxWidth={760}>
      <View style={{ gap: 8 }} testID="tape-performed">
        <T v="eb" accessibilityRole="header">{nights ? [countWord(nights, 'night', 'nights'), killedWord(killed)].filter(Boolean).join(' · ') : 'Never on tape'}</T>
        {s.performed.map((x, i) => <MomentRow key={x.id} m={x} showBit={false} testID={`tape-performed-${i}`} />)}
      </View>
      {!!takes.length && (
        <View style={{ gap: 8 }} testID="tape-filmed">
          <T v="eb" accessibilityRole="header">{countWord(takes.length, 'take filmed', 'takes filmed')}</T>
          <RecRows recs={takes} testID="tape-take" />
        </View>
      )}
      {!!s.like.moments.length && (
        <View style={{ gap: 8 }} testID="tape-echoes">
          <T v="eb" accessibilityRole="header">Sounds like it, on tape</T>
          {s.like.moments.map((x, i) => (
            <View key={x.id} style={{ gap: 4 }}>
              <MomentRow m={x} testID={`tape-echo-${i}`} />
              {!x.joke && <Btn small v="ghost" testID={`tape-echo-link-${i}`} label="That’s this bit · link it" onPress={() => linkMoment(x.id)} />}
            </View>
          ))}
        </View>
      )}
      {!!s.like.bits.length && (
        <View style={{ gap: 8 }} testID="tape-like">
          <T v="eb" accessibilityRole="header">Sounds like these bits</T>
          {s.like.bits.map((b, i) => (
            <Pressable key={b.id} testID={`tape-like-${i}`} accessibilityRole="button" accessibilityLabel={`${b.title}. ${closeness(b.distance) || 'Shares its words'}. Open`}
              onPress={() => router.push(`/joke/${enc(b.id)}` as any)}
              style={(st: any) => ({ minHeight: 52, paddingVertical: 8, paddingHorizontal: 12, borderRadius: 12, gap: 2, backgroundColor: st.pressed || st.hovered ? C.card2 : C.card })}>
              <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8 }}>
                <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ flex: 1, fontFamily: F.semi, fontSize: 15, color: C.text }}>{shortNameOf(b.title, b.short_name)}</Text>
                <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 12, color: C.mut2 }}>{closeness(b.distance) || 'shares its words'}</Text>
              </View>
              {!!b.why && <Text numberOfLines={2} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 20, color: C.mut2 }}>{oneLine(b.why)}</Text>}
              <View style={{ flexDirection: 'row', justifyContent: 'flex-end' }}>
                <Btn small v="ghost" testID={`tape-like-merge-${i}`} label="Same bit · merge it in" onPress={() => mergeLike(b)} />
              </View>
            </Pressable>
          ))}
        </View>
      )}
      {!nights && !takes.length && !s.like.moments.length && !s.like.bits.length && <T v="mut">Nothing on tape sounds like it{s.vectors ? '' : ' (by its words; the model is off)'}.</T>}
    </Screen>
  );
}
