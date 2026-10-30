// Tonight (MixHome, docs/UX_2026-10.md): the home tab. The date line, "N to sort" (the morning inbox), "+" (the create
// sheet), tonight's set list as one colour bar (a segment per bit, red where a transition is a gap), three questions
// to ask his material, and the one input bar. One request: GET /api/tonight/screen; changes arrive over /api/live.
import React, { useCallback, useState } from 'react';
import { PixelRatio, Platform, Pressable, Text, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { useApi } from '@/lib/api';
import { mmss } from '@/rules/format';
import { TonightScreen, useOutbox } from '@/lib/say';
import { CreateSheet } from '@/lib/CreateSheet';
import { C, F } from '@/lib/theme';
import { tonightTitleRow } from '@/rules/layoutRules';
import { noteWorker } from '@/lib/worker';
import { Avatar, Icon, Skeleton, T, useUI, useWide } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

/** The set as one bar: a segment per bit, width = its length, red = a gap before it (no link to the bit before). */
export function SetBar({ segments, height = 10, testID }: { segments: { short_name: string; duration_s: number; gap: boolean }[]; height?: number; testID?: string }) {
  if (!segments.length) return <View testID={testID} style={{ height, borderRadius: height / 2, backgroundColor: C.surface2 }} />;
  return (
    <View testID={testID} style={{ flexDirection: 'row', gap: 3, height, borderRadius: height / 2, overflow: 'hidden' }}>
      {segments.map((sg, i) => (
        <View key={i} testID={`${testID}-seg-${i}`} accessibilityLabel={`${sg.short_name}${sg.gap ? ', gap before it' : ''}`}
          style={{ flex: Math.max(10, sg.duration_s || 30), backgroundColor: sg.gap ? C.gap : C.amber, opacity: sg.gap ? 1 : 0.7 + 0.3 * ((i % 3) / 2) }} />
      ))}
    </View>
  );
}

export default function Tonight() {
  const wide = useWide();
  const ui = useUI();
  const s = useApi<TonightScreen>('/api/tonight/screen', { poll: 30000 });
  const queued = useOutbox();
  const [create, setCreate] = useState(false);
  const d = s.data;
  const now = new Date();
  const b = d?.board || null;
  React.useEffect(() => { noteWorker(d?.worker); }, [d?.worker?.online, d?.worker?.last_seen]); // eslint-disable-line react-hooks/exhaustive-deps
  React.useEffect(() => { ui.setToSort(d?.to_sort || 0); }, [d?.to_sort]); // eslint-disable-line react-hooks/exhaustive-deps

  // the conversation screen sends it (and keeps it on this phone when Punchable can't be reached)
  const onSay = useCallback((text: string) => router.push({ pathname: '/say', params: { text, screen: 'tonight' } } as any), []);
  // Owner decision 2026-10-28: no input box on the tab screens. Asking lives in the system search field under the
  // title (HIG/search-fields), typed only; new bits come from "+".
  const [askText, setAskText] = useState('');
  const search = Platform.OS !== 'web' ? {
    placeholder: 'Ask your material', placement: 'automatic' as const, hideWhenScrolling: true, autoCapitalize: 'sentences' as const, tintColor: C.amber,
    onChangeText: (e: any) => setAskText(e.nativeEvent.text), onSearchButtonPress: () => { const t = askText.trim(); if (t) onSay(t); }, onCancelButtonPress: () => setAskText(''),
  } : undefined;

  // HIG large-title shape (docs/design/HIG.md §0): row 1 is the bar (the date, "+", the face); row 2 is the title with
  // the "N to sort" pill, which stacks under the title at text sizes >= 1.3x. No Ask button: the input bar IS Ask.
  const titleRow = tonightTitleRow(PixelRatio.getFontScale());
  const native = Platform.OS !== 'web';
  const openCreate = () => (native ? router.push('/create' as any) : setCreate(true));
  const dateLine = `${DAYS[now.getDay()]}${d?.latest_set?.date === d?.date ? ' · set tonight' : ''}`;
  const header = (
    <View testID="tonight-header" style={{ paddingHorizontal: wide ? 32 : 20, paddingTop: 4, paddingBottom: 4, gap: 2 }}>
      <View testID="tonight-bar" style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44 }}>
        <T v="eb" testID="tonight-date" numberOfLines={1} style={{ flex: 1, minWidth: 0, color: C.mut2 }}>{DAYS[now.getDay()]}{d?.latest_set?.date === d?.date ? ' · set tonight' : ''}</T>
        <Pressable testID="create" accessibilityRole="button" accessibilityLabel="New" onPress={openCreate} hitSlop={4}
          style={{ width: 44, height: 44, borderRadius: 22, backgroundColor: C.card, alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
          <Icon name="plus" size={20} color={C.amber} sw={2.4} />
        </Pressable>
        <Avatar ask={false} />
      </View>
      <View testID={`tonight-title-${titleRow.direction}`} style={{ flexDirection: titleRow.direction, alignItems: titleRow.direction === 'row' ? 'center' : 'flex-start', gap: 8 }}>
        <Text numberOfLines={1} maxFontSizeMultiplier={1.3} accessibilityRole="header" style={{ flexShrink: 1, minWidth: 0, fontFamily: F.black, fontSize: 34, letterSpacing: -0.7, color: C.text }}>Tonight</Text>
        <Pressable testID="to-sort" accessibilityRole="button" accessibilityLabel={`${d?.to_sort || 0} to sort. Open the inbox`} onPress={() => router.push('/inbox' as any)}
          style={{ flexShrink: 0, minHeight: 44, paddingHorizontal: 12, borderRadius: 22, backgroundColor: C.card, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: d?.to_sort ? C.amber : C.borderStrong }} />
          {d ? <Text maxFontSizeMultiplier={1.4} numberOfLines={1} style={{ fontFamily: F.semi, fontSize: 14, color: C.text }}>{d.to_sort} to sort</Text> : <View style={{ width: 54, height: 12, borderRadius: 6, backgroundColor: C.surface2 }} accessibilityLabel="Loading" />}
        </Pressable>
      </View>
    </View>
  );

  return (
    <Screen title="Tonight" testID="tonight" header={native ? undefined : header} largeTitle={native} search={search as any} onRefresh={s.reload}>
      {native && (
        // HIG/toolbars: "+" and the face in the bar; the title stays short so there is room for them
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button icon="plus" accessibilityLabel="New" onPress={openCreate}>New</Stack.Toolbar.Button>
          <Stack.Toolbar.View><Avatar ask={false} /></Stack.Toolbar.View>
        </Stack.Toolbar>
      )}
      {native && (
        <View style={{ gap: 2 }}>
          <T v="eb" testID="tonight-date" style={{ color: C.mut2 }}>{dateLine}</T>
          {/* HIG/lists-and-tables: a disclosure indicator reveals the next level (the inbox), not a pill in the bar */}
          <Pressable testID="to-sort" accessibilityRole="button" accessibilityLabel={`${d?.to_sort || 0} to sort. Open the inbox`} onPress={() => router.push('/inbox' as any)}
            style={{ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
            <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: d?.to_sort ? C.amber : C.borderStrong }} />
            {d ? <Text maxFontSizeMultiplier={1.4} style={{ flex: 1, fontFamily: F.semi, fontSize: 16, color: C.text }}>{d.to_sort} to sort</Text> : <View style={{ flex: 1, height: 12, borderRadius: 6, backgroundColor: C.surface2 }} accessibilityLabel="Loading" />}
            <Icon name="chevron" size={18} color={C.dim} />
          </Pressable>
        </View>
      )}
      {!!queued.length && <T v="small" testID="say-queued" style={{ color: C.amber }}>{queued.length} saved on this phone, sending when Punchable is back…</T>}
      {s.rawError && !d && <T v="mut" testID="tonight-error">Can’t reach Punchable: {s.rawError}</T>}
      {!d && !s.rawError && <Skeleton lines={3} height={20} testID="tonight-skeleton" />}
      <Pressable testID="tonight-set" accessibilityRole="button" accessibilityLabel={b ? `${b.title}, ${b.runtime}. Open the set list` : 'No set list yet. Make one'}
        onPress={() => (b ? router.push(`/list/${encodeURIComponent(b.id)}` as any) : openCreate())}
        style={{ padding: 16, borderRadius: 20, backgroundColor: C.card, gap: 10 }}>
        <View style={{ flexDirection: 'row', justifyContent: 'space-between', alignItems: 'baseline' }}>
          <Text numberOfLines={1} style={{ flex: 1, fontFamily: F.bold, fontSize: 15, color: C.text }}>{b ? `Your ${Math.round(b.target_s / 60)} for tonight` : 'No set list yet'}</Text>
          <Text testID="tonight-runtime" style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{b ? mmss(b.runtime_s) : ''}</Text>
        </View>
        <SetBar segments={b?.segments || []} testID="tonight-bar" />
        <Text numberOfLines={2} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>
          {b ? b.segments.map((sg, i) => (
            <Text key={i}>{i ? ' · ' : ''}{sg.gap ? <Text style={{ color: C.gapText }}>gap · </Text> : null}{sg.short_name}</Text>
          )) : 'Tap + to start one, or say "new 7 for Friday".'}
        </Text>
      </Pressable>

      <View style={{ gap: 8 }}>
        <Text style={{ fontFamily: F.bold, fontSize: 20, color: C.text, marginTop: 10 }}>Ask your material</Text>
        {(d?.questions || ['What’s my strongest closer right now?']).map((q, i) => (
          <Pressable key={q} testID={`ask-suggest-${i}`} accessibilityRole="button" onPress={() => onSay(q)}
            style={(st: any) => ({ padding: 14, paddingHorizontal: 16, borderRadius: 16, backgroundColor: st.hovered ? C.card : C.card2, flexDirection: 'row', alignItems: 'center', gap: 12 })}>
            <Icon name="chat" size={18} color={C.amber} />
            <Text style={{ flex: 1, fontFamily: F.medium, fontSize: 15, color: C.text }}>{q}</Text>
          </Pressable>
        ))}
      </View>
      {d?.latest_set && (
        <Pressable testID="tonight-last-set" accessibilityRole="button" onPress={() => router.push(`/set/${encodeURIComponent(d.latest_set!.id)}` as any)} style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44 }}>
          <T v="mut" style={{ flex: 1 }}>Last set: {d.latest_set.title || d.latest_set.date}{d.latest_set.reviewed === false ? ' · not reviewed yet' : ''}</T>
          <T style={{ color: C.amber }}>Open ›</T>
        </Pressable>
      )}
      {!native && <CreateSheet visible={create} onClose={() => setCreate(false)} />}
    </Screen>
  );
}
