// "Same idea, N notes · Merge": the app's spot for the merge module (creative/merge.py, api-dev/merge_api.py, owned by
// another agent; contract in docs/API_CONTRACT.md "Merge"). Renders nothing until GET /api/merge/candidates answers.
//   <MergeHint jokeId=…/>  on the joke map: this joke's same-idea group, one tap to merge (Undo in the snackbar)
//   <MergeGroups/>         in the Notebook: how many ideas he wrote more than once, each one tap from its map
import React, { useEffect, useState } from 'react';
import { Pressable, View } from 'react-native';
import { router } from 'expo-router';
import { ApiError, get, post } from './api';
import { C } from './theme';
import { AIBadge, List, ListRow, Sheet, T, s, useUI } from './ui';

type Member = { joke_id: string; title: string; first_date?: string | null; last_date?: string | null; nights?: number };
type Group = { id: string; main: string; main_title: string; main_why?: string; members: Member[]; reasons?: string[] };
const enc = encodeURIComponent;
const short = (t?: string) => (t || '').split(' (')[0];

async function quiet<T>(p: Promise<T>): Promise<T | null> {
  try {
    return await p;
  } catch (e) {
    if (e instanceof ApiError) return null; // 404 = not served here yet; anything else: say nothing on this screen
    return null;
  }
}

export function MergeHint({ jokeId, onMerged }: { jokeId: string; onMerged?: (mainId: string) => void }) {
  const ui = useUI();
  const [g, setG] = useState<{ for: string; group: Group | null } | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    let alive = true;
    quiet(get<{ group: Group | null }>(`/api/merge/candidates?joke=${enc(jokeId)}`)).then((r) => alive && setG({ for: jokeId, group: r?.group || null }));
    return () => { alive = false; };
  }, [jokeId]);
  const group = g?.for === jokeId ? g.group : null;
  if (!group || group.members.length < 2) return null;
  const others = group.members.filter((m) => m.joke_id !== group.main).map((m) => m.joke_id);
  const doMerge = async () => {
    setOpen(false);
    try {
      const r = await post<{ merge_id: string }>('/api/merge', { main: group.main, others, group: group.id, reason: 'merged from the joke map' });
      setG({ for: jokeId, group: null });
      ui.snack(`Merged into ${short(group.main_title)}`, async () => { await post('/api/merge/undo', { merge_id: r.merge_id }); onMerged?.(jokeId); });
      onMerged?.(group.main);
    } catch (e: any) {
      ui.snack(`Couldn't merge: ${e.message}`);
    }
  };
  return (
    <>
      <Pressable testID="merge-hint" accessibilityRole="button" onPress={() => setOpen(true)} style={[s.inner, s.row, { minHeight: 48 }]}>
        <T v="semi" style={{ flex: 1 }}>Same idea, {group.members.length} notes</T>
        <T style={{ color: C.amber }}>Merge ›</T>
      </Pressable>
      <Sheet visible={open} onClose={() => setOpen(false)} label="Same idea" testID="merge-sheet">
        <View style={[s.row, { marginBottom: 6 }]}><T v="h2" style={{ flex: 1 }}>Same idea, {group.members.length} notes</T><AIBadge /></View>
        {(group.reasons || []).slice(0, 2).map((x, i) => <T key={i} v="mut">{x}</T>)}
        <List>
          {group.members.map((m) => (
            <ListRow key={m.joke_id} title={`${m.joke_id === group.main ? '★ ' : ''}${short(m.title)}`} sub={[m.first_date?.slice(0, 4), m.nights ? `${m.nights} ${m.nights === 1 ? 'night' : 'nights'}` : null].filter(Boolean).join(' · ')} chevron={false} />
          ))}
        </List>
        {!!group.main_why && <T v="small" style={{ marginVertical: 6 }}>Keeps ★ {short(group.main_title)}: {group.main_why}</T>}
        <Pressable testID="merge-go" accessibilityRole="button" onPress={doMerge} style={{ minHeight: 48, borderRadius: 12, backgroundColor: C.amber, alignItems: 'center', justifyContent: 'center', marginTop: 8 }}>
          <T v="semi" style={{ color: C.bg }}>Merge into one joke</T>
        </Pressable>
        <T v="small" style={{ marginTop: 8 }}>Every version stays, word for word, under its date. Undo puts them back.</T>
      </Sheet>
    </>
  );
}

export function MergeGroups() {
  const [d, setD] = useState<{ groups: Group[] } | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    let alive = true;
    quiet(get<{ groups: Group[] }>('/api/merge/candidates')).then((r) => alive && setD(r));
    return () => { alive = false; };
  }, []);
  const groups = d?.groups || [];
  if (!groups.length) return null;
  return (
    <>
      <Pressable testID="merge-groups" accessibilityRole="button" onPress={() => setOpen(true)} style={[s.inner, s.row, { minHeight: 48 }]}>
        <T v="semi" style={{ flex: 1 }}>Same idea, written more than once: {groups.length}</T>
        <T style={{ color: C.amber }}>Look ›</T>
      </Pressable>
      <Sheet visible={open} onClose={() => setOpen(false)} label="Same idea" testID="merge-groups-sheet">
        <View style={[s.row, { marginBottom: 6 }]}><T v="h2" style={{ flex: 1 }}>Same idea, more than once</T><AIBadge /></View>
        <List>
          {groups.slice(0, 40).map((g) => (
            <ListRow key={g.id} title={short(g.main_title)} sub={`Same idea, ${g.members.length} notes`} onPress={() => { setOpen(false); router.push(`/joke/${enc(g.main)}` as any); }} />
          ))}
        </List>
      </Sheet>
    </>
  );
}
