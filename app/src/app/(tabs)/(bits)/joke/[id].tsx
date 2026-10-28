// A bit (MixBit, docs/UX_2026-10.md): the versions as tabs (newest first), the text, the bird's-eye map, Punch up ·
// Riff · Merge · To a set, and its own input bar (a question about this bit, or a new line for it). Two requests:
// the joke (/api/jokes/<id>) and its map (/api/jokes/<id>/map). The old joke map stays at /map/<id>, the old
// workspace (stage, numbers, brainstorm) at /bit/<id>.
import React, { useCallback, useMemo, useState } from 'react';
import { Platform, Pressable, ScrollView, Share, Text, TextInput, View, useWindowDimensions } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { decide, undo, useApi } from '@/lib/api';
import { useBoards } from '@/lib/boards';
import { MergeHint } from '@/lib/MergeHint';
import { GROUP_COLOR, Radial } from '@/lib/jokemap';
import { JokeMap, MapGroupKey, monthYear, quoteText } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { SayBar } from '@/lib/SayBar';
import { C, F } from '@/lib/theme';
import { haptics } from '@/lib/haptics';
import { Btn, Empty, ErrorCard, MenuItem, MenuSheet, Sheet, SheetHead, Skeleton, StageWord, T, TextBtn, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

type Version = { key: string; label: string; date: string | null; text: string; doc?: string | null };

function versionsOf(d: any): Version[] {
  const out: Version[] = [];
  for (const g of d?.versions || []) {
    for (const u of g.units || []) {
      if (!u.text) continue;
      out.push({ key: u.id || `${g.date}-${out.length}`, label: '', date: u.date || g.date || null, text: u.text, doc: u.doc_title });
    }
  }
  if (!out.length && d?.joke?.summary && d?.yours) out.push({ key: 'idea', label: '', date: d.joke.first_seen || null, text: d.joke.summary });
  out.sort((a, b) => (b.date || '').localeCompare(a.date || ''));
  return out.map((v, i) => ({ ...v, label: `v${out.length - i}${v.date ? ` · ${monthYear(v.date)}` : ''}` }));
}

export default function BitPage() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const wide = useWide();
  const boards = useBoards();
  const { width } = useWindowDimensions();
  const d = useApi<any>(`/api/jokes/${encodeURIComponent(id)}`, { poll: 30000 });
  const m = useApi<JokeMap>(`/api/jokes/${encodeURIComponent(id)}/map?cap=6`);
  const [vi, setVi] = useState(0);
  const [rename, setRename] = useState(false);
  const [why, setWhy] = useState<{ title: string; why?: string; g: MapGroupKey } | null>(null);
  const [menu, setMenu] = useState<MenuItem[] | null>(null);
  const versions = useMemo(() => versionsOf(d.data), [d.data]);
  const J = d.data?.joke;
  const title = (J?.title || '').split(' (')[0] || J?.title || 'Bit';
  const shortName = shortNameOf(J?.title || '', J?.short_name);
  const onSay = useCallback((text: string) => router.push({ pathname: '/say', params: { text, joke: id, screen: 'bit' } } as any), [id]);
  const native = Platform.OS !== 'web';
  const edit = () => router.push(`/bit/${encodeURIComponent(id)}` as any);
  const retire = async () => {
    try {
      const w = await decide({ target: 'node', action: 'edit', id, after: { stage: 'retired' } });
      d.reload();
      ui.snack('Retired: resting, not gone', async () => { await undo(w.indexes); d.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't retire: ${e.message}`);
    }
  };

  const saveShort = async (v: string) => {
    setRename(false);
    try {
      const w = await decide({ target: 'node', action: 'edit', id, after: { short_name: v.trim() || null } });
      d.reload();
      ui.snack(v.trim() ? `“${v.trim()}” on your set lists` : 'Back to the default name', async () => { await undo(w.indexes); d.reload(); });
    } catch (e: any) {
      ui.snack(`Couldn't rename: ${e.message}`);
    }
  };

  if (d.rawError && !d.data) return <Screen title="Bit" back><ErrorCard error={String(d.rawError)} onRetry={d.reload} /></Screen>;
  if (!d.data) return <Screen title="Bit" back><Skeleton lines={6} height={18} testID="bit-skeleton" /></Screen>;
  const v = versions[Math.min(vi, Math.max(0, versions.length - 1))];
  const mapW = Math.min(width - (wide ? 64 : 32), 720);

  return (
    <Screen title={shortName} back testID="bitpage" maxWidth={760}
      right={native ? undefined : <TextBtn testID="bit-edit" label="Edit" onPress={edit} />}
      bar={<SayBar onSay={onSay} placeholder={`Ask about ${shortName}, or add a line`} hint={null} testID="bit-say" />}>
      {native && (
        // HIG/toolbars: "Edit" as text (not well represented by a symbol); the rest in a menu, Retire last and destructive
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button accessibilityLabel="Edit the bit" onPress={edit}>Edit</Stack.Toolbar.Button>
          <Stack.Toolbar.Menu icon="ellipsis.circle" accessibilityLabel="More">
            <Stack.Toolbar.MenuAction icon="pencil" onPress={() => setRename(true)}>Rename short name…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="plus" disabled={!boards.current} onPress={() => boards.add(id, title)}>{`Add to ${boards.current?.title || "tonight's set"}`}</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="list.bullet" onPress={() => boards.pick(id, title)}>Pick a set list…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="arrow.triangle.merge" onPress={() => router.push('/merge' as any)}>Merge with…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="square.and.arrow.up" onPress={() => Share.share({ message: [title, v?.text || ''].join('\n\n').trim(), title }).catch(() => ui.snack('Sharing is not available here'))}>Share…</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="archivebox" destructive onPress={retire}>Retire</Stack.Toolbar.MenuAction>
          </Stack.Toolbar.Menu>
        </Stack.Toolbar>
      )}
      <View style={{ gap: 6 }}>
        {/* HIG/typography Title 1 (28 pt) for the full title; the bar carries the short name */}
        <Text testID="bit-title" accessibilityRole="header" style={{ fontFamily: F.black, fontSize: 28, letterSpacing: -0.5, color: C.text }}>{title}</Text>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
          <StageWord stage={J?.stage} ai={!!d.data?.ai?.stage_ai && !d.data?.ai?.stage_confirmed} />
          <Pressable testID="bit-shortname" accessibilityRole="button" accessibilityLabel={`On set lists: ${shortName}. Rename`} onPress={() => setRename(true)} hitSlop={6} style={{ minHeight: 44, justifyContent: 'center' }}>
            <T v="small">On the list: <Text style={{ color: C.amber }}>{shortName}</Text></T>
          </Pressable>
          {!!d.data?.rooms?.sets && <T v="small">{d.data.rooms.sets} set{d.data.rooms.sets === 1 ? '' : 's'}</T>}
        </View>
      </View>

      {versions.length > 1 && (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ gap: 6 }} testID="bit-versions">
          {versions.map((x, i) => (
            <Pressable key={x.key} testID={`bit-v-${i}`} accessibilityRole="tab" accessibilityState={{ selected: i === vi }} onPress={() => setVi(i)}
              style={{ minHeight: 36, paddingHorizontal: 12, borderRadius: 18, backgroundColor: i === vi ? C.text : C.card, justifyContent: 'center' }}>
              <Text style={{ fontFamily: F.semi, fontSize: 13, color: i === vi ? C.bg : C.mut2 }}>{x.label}{i === 0 ? ' · newest' : ''}</Text>
            </Pressable>
          ))}
        </ScrollView>
      )}
      <View testID="bit-text" style={{ padding: 16, borderRadius: 16, backgroundColor: C.card, gap: 6 }}>
        {v ? <Text selectable style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text }}>{v.text}</Text> : <T v="mut">No written version yet. Say a line below and it lands here.</T>}
        {!!v?.doc && <T v="small">{v.doc}{v.date ? ` · ${monthYear(v.date)}` : ''}</T>}
      </View>

      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
        <Btn small testID="bit-punchup" label="Punch up" onPress={() => router.push({ pathname: `/bit/${encodeURIComponent(id)}`, params: { tab: 'brainstorm' } } as any)} />
        <Btn small testID="bit-riff" label="Riff" onPress={() => router.push({ pathname: '/write', params: { idea: v?.text?.slice(0, 300) || title } } as any)} />
        <Btn small testID="bit-toset" v="add" label="To a set" onPress={() => { haptics.success(); boards.add(id, title); }}
          onLongPress={() => setMenu([
            { label: `Add to ${boards.current?.title || 'the current set list'}`, onPress: () => boards.add(id, title), disabled: !boards.current },
            { label: 'Pick a set list…', onPress: () => boards.pick(id, title) },
          ])} />
      </View>
      <MergeHint jokeId={id} onMerged={(main) => (main === id ? d.reload() : router.replace(`/joke/${encodeURIComponent(main)}` as any))} />

      <View style={{ gap: 6 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center' }}>
          <T v="eb" style={{ flex: 1 }}>Map</T>
          <T v="small">tap a bit to see why</T>
          <TextBtn label="Full map ›" testID="bit-fullmap" onPress={() => router.push(`/map/${encodeURIComponent(id)}` as any)} />
        </View>
        {m.data ? (
          m.data.groups.some((g) => g.items.length)
            ? <Radial map={m.data} width={mapW} onOpen={(b, g) => setWhy({ title: b.title, why: b.why || b.stage_line || (b.quote ? `“${quoteText(b)}”` : ''), g })} />
            : <T v="mut">Nothing connected to it yet.</T>
        ) : <T v="mut">Drawing the map…</T>}
        {!!m.data && (
          <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 12 }}>
            {m.data.groups.filter((g) => g.items.length).map((g) => (
              <View key={g.key} style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}><View style={{ width: 10, height: 3, backgroundColor: GROUP_COLOR[g.key] }} /><T v="small">{g.title}</T></View>
            ))}
          </View>
        )}
      </View>

      <Sheet visible={!!why} onClose={() => setWhy(null)} label="Why" testID="why-sheet">
        {why && (
          <View style={{ gap: 8 }}>
            <T v="h2">{why.title}</T>
            <T v="mut">{why.why || 'Connected on the map.'}</T>
            <Btn label="Open it" onPress={() => { const b = m.data!.groups.flatMap((g) => g.items).find((x) => x.title === why.title); setWhy(null); if (b) router.push(`/joke/${encodeURIComponent(b.joke_id)}` as any); }} />
          </View>
        )}
      </Sheet>
      <Sheet visible={rename} onClose={() => setRename(false)} label="Short name" testID="bit-rename-sheet">
        <SheetHead title="Short name" onCancel={() => setRename(false)} testID="bit-rename" />
        <ShortName value={shortName} title={J?.title || ''} onSave={saveShort} />
      </Sheet>
      <MenuSheet visible={!!menu} items={menu || []} onClose={() => setMenu(null)} title={title} testID="bit-menu" />
    </Screen>
  );
}

function ShortName({ value, title, onSave }: { value: string; title: string; onSave: (v: string) => void }) {
  const [v, setV] = useState(value);
  return (
    <View style={{ gap: 8 }}>
      <T v="mut">What you’d write on the paper list. Up to 24 letters.</T>
      <TextInput testID="bit-shortname-input" value={v} onChangeText={setV} maxLength={24} autoFocus placeholder={shortNameOf(title)} placeholderTextColor={C.dim}
        style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.bold, fontSize: 18 }} />
      <View style={{ flexDirection: 'row', gap: 8 }}>
        <Btn testID="bit-shortname-save" label="Save" v="primary" style={{ flex: 1 }} disabled={!v.trim()} onPress={() => onSave(v)} />
        <Btn label="Default" v="ghost" onPress={() => onSave('')} />
      </View>
    </View>
  );
}
