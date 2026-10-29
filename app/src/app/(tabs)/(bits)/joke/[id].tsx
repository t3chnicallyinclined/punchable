// A bit (MixBit, docs/UX_2026-10.md): the versions as tabs (newest first), the text, the bird's-eye map, Punch up ·
// Riff · Merge · To a set, and its own input bar (a question about this bit, or a new line for it). Two requests:
// the joke (/api/jokes/<id>) and its map (/api/jokes/<id>/map). The old joke map stays at /map/<id>, the old
// workspace (stage, numbers, brainstorm) at /bit/<id>. The WORKBENCH (2026-10-29, owner: "open a bit, edit and speak
// to my material, brainstorm, then organize the joke live"): Edit writes a NEW version (the old ones stay, undoable;
// POST /api/jokes/<id>/versions); the input bar talks to a conversation pinned to this bit (lib/BitTalk) or, for a
// line, adds it as a version; "Elsewhere" rows merge a like bit into this one or link a taped moment to it.
// "Elsewhere" (a third request, /api/jokes/<id>/similar) answers
// "did I ever do this, and is it like anything?": the tapes it is on, the taped moments that sound like it but are
// not linked, and the bits that sound like it (api-dev/search_api.py; lib/MomentRow).
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Platform, Pressable, ScrollView, Share, Text, TextInput, View, useWindowDimensions } from 'react-native';
import { Stack, router, useLocalSearchParams } from 'expo-router';
import { decide, undo, useApi } from '@/lib/api';
import { useBoards } from '@/lib/boards';
import { MergeHint } from '@/lib/MergeHint';
import { BitTalk, useBitTalk } from '@/lib/BitTalk';
import { addVersion, mergeInto, undoMerge } from '@/lib/bitWork';
import { isQuestion } from '@/lib/say';
import { MomentRow } from '@/lib/MomentRow';
import { SimilarResult, closeness, oneLine } from '@/lib/searchRows';
import { GROUP_COLOR, Radial } from '@/lib/jokemap';
import { JokeMap, MapGroupKey, monthYear, quoteText } from '@/lib/simple';
import { shortNameOf } from '@/lib/say';
import { useSayBar } from '@/lib/SayBar';
import { C, F } from '@/lib/theme';
import { haptics } from '@/lib/haptics';
import { actionSheet } from '@/lib/actionSheet';
import { afterWrite } from '@/lib/feedback';
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
  const p = useLocalSearchParams<{ id: string; edit?: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const ui = useUI();
  const wide = useWide();
  const boards = useBoards();
  const { width } = useWindowDimensions();
  const d = useApi<any>(`/api/jokes/${encodeURIComponent(id)}`, { poll: 30000 });
  const m = useApi<JokeMap>(`/api/jokes/${encodeURIComponent(id)}/map?cap=6`);
  const sim = useApi<SimilarResult>(`/api/jokes/${encodeURIComponent(id)}/similar?limit=5`);
  const [vi, setVi] = useState(0);
  const [rename, setRename] = useState(false);
  const [why, setWhy] = useState<{ title: string; why?: string; g: MapGroupKey } | null>(null);
  const [menu, setMenu] = useState<MenuItem[] | null>(null);
  const [editing, setEditing] = useState<string | null>(null);   // the text being edited (a new version on Save)
  // opened from Bits "+": a fresh document, the editor open and focused before anything else
  const [openedFresh, setOpenedFresh] = useState(false);
  useEffect(() => {
    if (p.edit === '1' && d.data && !openedFresh && editing == null) { setEditing(''); setOpenedFresh(true); }
  }, [p.edit, d.data, openedFresh, editing]);
  const [saving, setSaving] = useState(false);
  const versions = useMemo(() => versionsOf(d.data), [d.data]);
  const reloadAll = useCallback(() => { d.reload(); sim.reload(); }, [d.reload, sim.reload]);
  const talk = useBitTalk(id, reloadAll);
  const J = d.data?.joke;
  const title = (J?.title || '').split(' (')[0] || J?.title || 'Bit';
  const shortName = shortNameOf(J?.title || '', J?.short_name);
  // the bar: a question goes to the conversation about this bit; a line becomes its newest version
  const onSay = useCallback(async (text: string) => {
    const t = text.trim();
    if (!t) return;
    if (isQuestion(t)) return talk.askIt(t);
    try {
      const r = await addVersion(id, t, 'a line said on the bit page');
      haptics.success();
      setVi(0);
      reloadAll();
      ui.snack('Added as the newest version', async () => { await undo(r.indexes); reloadAll(); });
    } catch (e: any) {
      ui.snack(`Couldn’t add it: ${e.message}`);
    }
  }, [id, talk, reloadAll, ui]);
  const saveEdit = async () => {
    const t = (editing || '').trim();
    if (!t || saving) return;
    if (t === (v?.text || '').trim()) { setEditing(null); return; }
    setSaving(true);
    try {
      const r = await addVersion(id, t, 'edited on the bit page');
      let indexes = r.indexes;
      if ((J?.title || '').startsWith('Untitled bit')) {
        // a fresh document takes its name from its first line (the first seven words, like a typed jot)
        const name = t.split(/\s+/).slice(0, 7).join(' ').replace(/[\s,.;:!?"']+$/, '').slice(0, 120) || 'New bit';
        try { const w = await decide({ target: 'node', action: 'edit', id, after: { title: name }, reason: 'named from its first line' }); indexes = [...indexes, ...w.indexes]; } catch { /* the version is saved either way */ }
      }
      haptics.success();
      setEditing(null);
      setVi(0);
      reloadAll();
      ui.snack(openedFresh ? 'Saved' : 'Saved as a new version · the old one stays', async () => { await undo(indexes); reloadAll(); });
    } catch (e: any) {
      ui.snack(`Couldn’t save: ${e.message}`);
    } finally {
      setSaving(false);
    }
  };
  const mergeLike = (b: { id: string; title: string; short_name?: string | null }) => {
    const name = shortNameOf(b.title, b.short_name);
    const go = async () => {
      try {
        const r = await mergeInto(id, [b.id], `merged from Elsewhere on ${shortName}`);
        haptics.success();
        reloadAll();
        ui.snack(`${name} is now part of this bit`, async () => { await undoMerge(r.merge_id); reloadAll(); });
      } catch (e: any) {
        ui.snack(`Couldn’t merge: ${e.message}`);
      }
    };
    if (native) actionSheet({ title: `Merge “${name}” into ${shortName}?`, message: 'Its versions and tapes move under this bit. You can undo it.', options: [{ label: 'Merge', onPress: go }, { label: 'Cancel', cancel: true }] });
    else go();
  };
  const linkMoment = async (pid: string) => {
    try {
      const w = await decide({ target: 'edge', action: 'add', after: { src: pid, dst: id, kind: 'performed_as' }, reason: 'linked from Elsewhere on the bit page' });
      haptics.success();
      reloadAll();
      ui.snack('Linked: that tape is this bit', async () => { await undo(w.indexes); reloadAll(); });
    } catch (e: any) {
      ui.snack(`Couldn’t link it: ${e.message}`);
    }
  };
  const native = Platform.OS !== 'web';
  const bar = useSayBar({ onSay, placeholder: `Ask about ${shortName}, or add a line`, hint: null, testID: 'bit-say', busy: talk.sending || talk.thinking });
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
      bar={bar}>
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
      {editing != null ? (
        <View testID="bit-edit" style={{ padding: 12, borderRadius: 16, backgroundColor: C.card, gap: 10, borderWidth: 1, borderColor: C.amber }}>
          <TextInput testID="bit-edit-input" value={editing} onChangeText={setEditing} multiline autoFocus textAlignVertical="top" accessibilityLabel="The bit's text"
            style={{ minHeight: 160, fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text }} />
          <View style={{ flexDirection: 'row', gap: 8, alignItems: 'center' }}>
            <T v="small" style={{ flex: 1 }}>Saves as a new version · the old one stays</T>
            <Btn small v="ghost" testID="bit-edit-cancel" label="Cancel" onPress={() => setEditing(null)} />
            <Btn small v="primary" testID="bit-edit-save" label={saving ? 'Saving…' : 'Save'} disabled={saving || !(editing || '').trim()} onPress={saveEdit} />
          </View>
        </View>
      ) : (
        <Pressable testID="bit-text" accessibilityRole="button" accessibilityLabel={v ? 'The bit’s text. Tap to edit' : 'No written version yet. Tap to write one'} onPress={() => setEditing(v?.text || '')}
          style={({ pressed }) => ({ padding: 16, borderRadius: 16, backgroundColor: pressed ? C.card2 : C.card, gap: 6 })}>
          {v ? <Text selectable style={{ fontFamily: F.body, fontSize: 17, lineHeight: 26, color: C.text }}>{v.text}</Text> : <T v="mut">No written version yet. Tap here to write one, or say a line below.</T>}
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            {!!v?.doc && <T v="small" style={{ flex: 1 }}>{v.doc}{v.date ? ` · ${monthYear(v.date)}` : ''}</T>}
            {!v?.doc && <View style={{ flex: 1 }} />}
            <T v="small" style={{ color: C.amber }}>Edit</T>
          </View>
        </Pressable>
      )}

      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
        <Btn small testID="bit-punchup" label="Punch up" onPress={() => router.push({ pathname: `/bit/${encodeURIComponent(id)}`, params: { tab: 'brainstorm' } } as any)} />
        <Btn small testID="bit-riff" label="Riff" onPress={() => router.push({ pathname: '/write', params: { idea: v?.text?.slice(0, 300) || title } } as any)} />
        <Btn small testID="bit-toset" v="add" label="To a set" onPress={() => afterWrite(() => boards.add(id, title), (ok) => ok, haptics)}
          onLongPress={() => {
            const items: MenuItem[] = [
              { label: `Add to ${boards.current?.title || 'the current set list'}`, onPress: () => boards.add(id, title), disabled: !boards.current },
              { label: 'Pick a set list…', onPress: () => boards.pick(id, title) },
            ];
            if (native) actionSheet({ title: 'To a set', options: [...items.filter((it) => !it.disabled).map((it) => ({ label: it.label, onPress: it.onPress })), { label: 'Cancel', cancel: true }] });
            else setMenu(items);
          }} />
      </View>
      <MergeHint jokeId={id} onMerged={(main) => (main === id ? d.reload() : router.replace(`/joke/${encodeURIComponent(main)}` as any))} />
      <BitTalk jokeId={id} shortName={shortName} talk={talk} />
      {!talk.chat && <T v="small">Ask anything about this bit below: “did I ever do this?”, “what else do I have like it?”, “tighten the second line”. Answers come with Keep as a line.</T>}

      <View style={{ gap: 8 }} testID="bit-elsewhere">
        <T v="eb">Elsewhere</T>
        {sim.data ? (
          <>
            {sim.data.performed.length ? (
              <View style={{ gap: 6 }}>
                <T v="small">On tape · {sim.data.performed.length} {sim.data.performed.length === 1 ? 'time' : 'times'}</T>
                {sim.data.performed.slice(0, 3).map((x, i) => <MomentRow key={x.id} m={x} showBit={false} testID={`bit-performed-${i}`} />)}
                {sim.data.performed.length > 3 && <T v="small">{sim.data.performed.length - 3} more in the numbers (Edit).</T>}
              </View>
            ) : <T v="mut">Never on tape.</T>}
            {!!sim.data.like.moments.length && (
              <View style={{ gap: 6 }}>
                <T v="small">Sounds like it, on tape (not linked to this bit)</T>
                {sim.data.like.moments.map((x, i) => (
                  <View key={x.id} style={{ gap: 4 }}>
                    <MomentRow m={x} testID={`bit-echo-${i}`} />
                    {!x.joke && <Btn small v="ghost" testID={`bit-echo-link-${i}`} label="That’s this bit · link it" onPress={() => linkMoment(x.id)} />}
                  </View>
                ))}
              </View>
            )}
            {!!sim.data.like.bits.length && (
              <View style={{ gap: 6 }}>
                <T v="small">Sounds like these bits</T>
                {sim.data.like.bits.map((b, i) => (
                  <Pressable key={b.id} testID={`bit-like-${i}`} accessibilityRole="button" accessibilityLabel={`${b.title}. ${closeness(b.distance) || 'Shares its words'}. Open`}
                    onPress={() => router.push(`/joke/${encodeURIComponent(b.id)}` as any)}
                    style={(s: any) => ({ minHeight: 52, paddingVertical: 8, paddingHorizontal: 12, borderRadius: 12, gap: 2, backgroundColor: s.pressed || s.hovered ? C.card2 : C.card })}>
                    <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8 }}>
                      <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ flex: 1, fontFamily: F.semi, fontSize: 15, color: C.text }}>{shortNameOf(b.title, b.short_name)}</Text>
                      <Text maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 12, color: C.mut2 }}>{closeness(b.distance) || 'shares its words'}</Text>
                    </View>
                    {!!b.why && <Text numberOfLines={2} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 14, lineHeight: 20, color: C.mut2 }}>{oneLine(b.why)}</Text>}
                    <View style={{ flexDirection: 'row', justifyContent: 'flex-end' }}>
                      <Btn small v="ghost" testID={`bit-like-merge-${i}`} label="Same bit · merge it in" onPress={() => mergeLike(b)} />
                    </View>
                  </Pressable>
                ))}
              </View>
            )}
            {!sim.data.like.moments.length && !sim.data.like.bits.length && <T v="mut">Nothing else sounds like it{sim.data.vectors ? '' : ' (by its words; the model is off)'}.</T>}
          </>
        ) : sim.rawError ? <T v="mut">Couldn’t look elsewhere right now.</T> : <T v="mut">Looking through your sets and bits…</T>}
      </View>

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
      <TextInput testID="bit-shortname-input" value={v} onChangeText={setV} maxLength={24} autoFocus placeholder={shortNameOf(title)} placeholderTextColor={C.dim} clearButtonMode="while-editing"
        style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.bold, fontSize: 18 }} />
      <View style={{ flexDirection: 'row', gap: 8 }}>
        <Btn testID="bit-shortname-save" label="Save" v="primary" style={{ flex: 1 }} disabled={!v.trim()} onPress={() => onSave(v)} />
        <Btn label="Default" v="ghost" onPress={() => onSave('')} />
      </View>
    </View>
  );
}
