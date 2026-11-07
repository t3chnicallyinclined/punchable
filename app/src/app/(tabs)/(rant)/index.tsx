// Rant (docs/design/SPEC-2026-11-06-rant-tab.md §2, slice 1): the first tab (R1; Tonight's parts moved to Sets and
// Bits, R2). A large title; the toolbar's "+" opens Write a rant (/rant/new), the face last. Top to bottom: Film a rant
// (the 60-pt red disc row: Film it on the newest draft, or a pick among several; never without a script), My topics
// (typed; the count of his bits that match; tap = Write a rant on it; swipe = Delete), Drafts (rants not filmed yet),
// Finished (the takes of rants filmed on this phone, in Photos). Trending is slice 2. The tab bar only navigates
// (R4): the record control is this screen's content. Requests: GET /api/topics, GET /api/rants (pk-api, Rust only),
// and /api/tonight/screen through lib/tonight so the Bits badge is right from launch.
import React, { useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import { Stack, router } from 'expo-router';
import { ApiError, useApi, useOnline } from '@/lib/api';
import { rant, useTopicCounts } from '@/lib/rant';
import { useTonight } from '@/lib/tonight';
import { useRecordings } from '@/lib/recordings';
import { actionSheet } from '@/lib/actionSheet';
import { haptics } from '@/lib/haptics';
import { SwipeRow } from '@/lib/SwipeRow';
import { RowMenu } from '@/lib/RowMenu';
import { C, F } from '@/lib/theme';
import { Avatar, Empty, Icon, IconBtn, List, ListRow, Skeleton, T, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { draftSub, filmTarget, finishedSub, splitRants, topicSub, type RantRow, type Topic } from '@/rules/rantRules';

const native = Platform.OS !== 'web';
const enc = encodeURIComponent;

/** Write a rant, on a topic when he tapped one. */
const writeRant = (t?: Topic | null) =>
  router.push({ pathname: '/rant/new', params: t ? { topic: t.text, topic_id: t.id } : {} } as any);

function SectionHead({ title, testID }: { title: string; testID?: string }) {
  return <T v="eb" testID={testID} accessibilityRole="header" style={{ marginTop: 14, marginBottom: 2 }}>{title}</T>;
}

export default function Rant() {
  const ui = useUI();
  const online = useOnline();
  useTonight(); // the Bits tab's "N to sort" badge, from launch (lib/tonight)
  const tp = useApi<{ topics: Topic[] }>('/api/topics');
  const rs = useApi<{ rants: RantRow[] }>('/api/rants');
  const takes = useRecordings();
  const topics = tp.data?.topics || [];
  const counts = useTopicCounts(topics);
  const { drafts, finished } = splitRants(rs.data?.rants || [], takes);
  const [typed, setTyped] = useState('');
  const [adding, setAdding] = useState(false);
  const refresh = () => Promise.all([tp.reload(), rs.reload()]);
  const fail = (what: string, e: any) => ui.snack(e instanceof ApiError ? `${what}: ${e.message}` : 'Can’t reach Punchable. Nothing changed.');

  const addTopic = async () => {
    const text = typed.trim();
    if (!text || adding) return;
    setAdding(true);
    try {
      await rant.addTopic(text);
      haptics.success();
      setTyped('');
      tp.reload();
    } catch (e: any) { fail('Couldn’t add it', e); }
    finally { setAdding(false); }
  };
  // Delete is undoable (HIG/undo-and-redo): Undo adds the same words back
  const delTopic = async (t: Topic) => {
    try {
      await rant.deleteTopic(t.id);
      tp.reload();
      ui.snack(`Deleted “${t.text}”`, async () => { await rant.addTopic(t.text); tp.reload(); });
    } catch (e: any) { fail('Couldn’t delete it', e); }
  };
  const film = () => {
    const f = filmTarget(drafts);
    if (f.k === 'none') return;
    if (f.k === 'one') return router.push(`/film/${enc(f.id)}` as any);
    actionSheet({ title: 'Film which rant?', options: [
      ...f.rows.map((r) => ({ label: r.short_name || r.title, onPress: () => router.push(`/film/${enc(r.id)}` as any) })),
      { label: 'Cancel', cancel: true },
    ] });
  };
  const loading = !tp.data && !rs.data && !tp.rawError && !rs.rawError;
  const nothing = !!tp.data && !!rs.data && !topics.length && !drafts.length && !finished.length;
  const filmable = drafts.length > 0;

  return (
    <Screen title="Rant" testID="rant" largeTitle={native} onRefresh={refresh}
      right={native ? undefined : <View style={{ flexDirection: 'row', alignItems: 'center' }}>
        <IconBtn testID="rant-new" label="Write a rant" onPress={() => writeRant()}><Icon name="plus" size={24} color={C.amber} sw={2.4} /></IconBtn>
        <Avatar />
      </View>}>
      {native && (
        // HIG/toolbars: the primary action trailing, the face last; no ⋯ (nothing else to do from here yet)
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button icon="plus" accessibilityLabel="Write a rant" accessibilityHint="Opens a blank rant to write" onPress={() => writeRant()}>Write a rant</Stack.Toolbar.Button>
          <Stack.Toolbar.View><Avatar ask={false} /></Stack.Toolbar.View>
        </Stack.Toolbar>
      )}
      {/* Film a rant: the record control is content, not a tab (R4); greyed with its reason when nothing is written */}
      <Pressable testID="rant-film" accessibilityRole="button" accessibilityState={{ disabled: !filmable }} disabled={!filmable}
        accessibilityLabel={filmable ? `Film a rant${drafts.length === 1 ? `: ${drafts[0].short_name}` : ''}` : 'Film a rant. Write a rant first'}
        onPress={film} style={({ pressed }) => ({ flexDirection: 'row', alignItems: 'center', gap: 14, minHeight: 76, paddingVertical: 8, opacity: !filmable ? 0.45 : pressed ? 0.7 : 1 })}>
        <View style={{ width: 60, height: 60, borderRadius: 30, borderWidth: 3, borderColor: C.text, alignItems: 'center', justifyContent: 'center' }}>
          <View style={{ width: 46, height: 46, borderRadius: 23, backgroundColor: C.rec }} />
        </View>
        <View style={{ flex: 1, minWidth: 0 }}>
          <Text maxFontSizeMultiplier={1.6} style={{ fontFamily: F.semi, fontSize: 17, color: C.text }}>Film a rant</Text>
          <T v="mut" numberOfLines={1}>{!filmable ? 'Write a rant first' : drafts.length === 1 ? drafts[0].short_name : `${drafts.length} drafts`}</T>
        </View>
      </Pressable>

      {loading && <Skeleton lines={4} height={20} testID="rant-skeleton" />}
      {nothing && <Empty text="Add a topic, or tap + to write a rant." />}

      <SectionHead title="My topics" testID="rant-topics-head" />
      <List>
        {topics.map((t) => (
          <SwipeRow key={t.id} testID={`topic-swipe-${t.id}`} trailing={{ label: 'Delete', destructive: true, onPress: () => void delTopic(t), testID: `topic-delete-${t.id}` }}>
            {/* HIG/accessibility: the swipe has a menu twin (long press: Delete), as the archived lists do */}
            <RowMenu href="/rant/new" items={[{ label: 'Delete', icon: 'trash', destructive: true, onPress: () => void delTopic(t), testID: `topic-menu-delete-${t.id}` }]}>
              <ListRow testID={`topic-${t.id}`} title={<T v="semi" numberOfLines={2}>{t.text}</T>} chevron={false}
                right={<T v="small">{topicSub(t, counts[t.id])}</T>} onPress={() => writeRant(t)} />
            </RowMenu>
          </SwipeRow>
        ))}
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 48 }}>
          <Icon name="plus" size={18} color={online ? C.amber : C.dim} />
          <TextInput testID="topic-add" value={typed} onChangeText={setTyped} placeholder={online ? 'Add a topic' : 'Add a topic (offline)'} placeholderTextColor={C.dim}
            editable={online} returnKeyType="done" onSubmitEditing={() => void addTopic()} submitBehavior="submit" accessibilityLabel="Add a topic"
            maxLength={200} maxFontSizeMultiplier={1.6} style={{ flex: 1, minHeight: 44, color: C.text, fontFamily: F.body, fontSize: 16 }} />
        </View>
      </List>

      {!!drafts.length && <SectionHead title="Drafts" testID="rant-drafts-head" />}
      <List>
        {drafts.map((r) => (
          <ListRow key={r.id} testID={`rant-draft-${r.id}`} title={<T v="semi" numberOfLines={1}>{r.short_name || r.title}</T>} sub={draftSub(r)}
            onPress={() => router.push(`/joke/${enc(r.id)}` as any)} />
        ))}
      </List>

      {!!finished.length && <SectionHead title="Finished" testID="rant-finished-head" />}
      <List>
        {finished.map(({ take, rant: r }) => (
          <ListRow key={take.id} testID={`rant-finished-${take.id}`} title={<T v="semi" numberOfLines={1}>{r.short_name || r.title}</T>} sub={finishedSub(take)}
            onPress={() => router.push(`/take/${enc(take.id)}` as any)} />
        ))}
      </List>
      {(tp.rawError && !tp.data) || (rs.rawError && !rs.data) ? <T v="mut" testID="rant-error">Can’t reach Punchable: {tp.rawError || rs.rawError}</T> : null}
    </Screen>
  );
}
