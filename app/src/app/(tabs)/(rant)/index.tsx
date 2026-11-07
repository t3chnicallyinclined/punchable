// Rant (SPEC 2026-11-06 §12.2, the creative space): the first tab opens on his work. A large title; the toolbar's "+"
// is New topic, the face last. Top to bottom: **Where he left off** (only when a topic has a rant: one card with the
// topic, the rant's last two lines in his words, "Rant v3 · Nov 6 · 0:58", Keep writing and Film it; its body opens the
// topic), **Topics** (last touched first; one sub line each, every number with a page behind it; tap pushes the topic;
// swipe or long press: Delete, Rename…), the rants no topic names (tap: the bit), and **Filmed** (one row, from the first
// take). Empty: "What do you want to rant about?" and New topic. The tab bar only navigates (R4); the red disc lives
// inside Film it. Requests: GET /api/topics, GET /api/rants (pk-api, Rust only), and /api/tonight/screen through
// lib/tonight so the Bits badge is right from launch. The tab's focus sends any script saved only on this phone.
import React, { useCallback, useEffect } from 'react';
import { Platform, Pressable, Text, View } from 'react-native';
import { Stack, router, useFocusEffect } from 'expo-router';
import { ApiError, useApi, useOnline } from '@/lib/api';
import { onTopicsChanged, rant, saveScript } from '@/lib/rant';
import { dropDraft, pendingDrafts } from '@/lib/scriptDraft';
import { scripts } from '@/lib/scriptStore';
import { useTonight } from '@/lib/tonight';
import { useRecordings } from '@/lib/recordings';
import { askText } from '@/lib/askText';
import { haptics } from '@/lib/haptics';
import { SwipeRow } from '@/lib/SwipeRow';
import { RowMenu } from '@/lib/RowMenu';
import { Icon } from '@/lib/Icon';
import { C, F } from '@/lib/theme';
import { Avatar, Btn, Empty, IconBtn, List, ListRow, Skeleton, T, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { PROSE_CAP, ROW_CAP, monthDay } from '@/rules/bitLines';
import { filmedList, leftOff, rantsWithoutTopic, runOf, takesOf, topicSub, type RantRow, type Take, type Topic } from '@/rules/rantRules';
import { fromDoc } from '@/rules/scriptRules';

const native = Platform.OS !== 'web';
const enc = encodeURIComponent;
const FOUND_RETRY_MS = 2500;

const openTopic = (t: Topic) => router.push({ pathname: '/topic/[topic]', params: { topic: t.id } } as any);
const newTopic = () => router.push('/new-topic' as any);

function SectionHead({ title, testID }: { title: string; testID?: string }) {
  return <T v="eb" testID={testID} accessibilityRole="header" style={{ marginTop: 14, marginBottom: 2 }}>{title}</T>;
}

let flushing = false;
/** Scripts saved only on this phone (offline, or the app died): sent when the tab shows, over the version they were
 *  written on; a draft of an older version than the server's is the server's loss to win (it is dropped). */
async function flushScripts(topics: Topic[]) {
  if (flushing) return;
  flushing = true;
  try {
    for (const d of await pendingDrafts()) {
      if (scripts.get(d.topic)) continue; // its page is open: that page saves it
      const t = topics.find((x) => x.id === d.topic);
      if (!t) continue;
      const serverUnit = t.unit || null;
      // a draft of an older version than the server's loses (a newer save wins); a first rant or "Start another rant"
      // (no rant yet on the draft) is a new bit
      if (d.rant && (t.rants || [t.rant_id]).every((r) => r !== d.rant)) { await dropDraft(d.topic); continue; }
      if (d.rant && d.rant === t.rant_id && serverUnit && serverUnit !== d.base) { await dropDraft(d.topic); continue; }
      try {
        await saveScript({ topicId: t.id, topicText: t.text, rantId: d.rant || null, lines: fromDoc(d.lines), saved: null });
        await dropDraft(d.topic);
      } catch { break; } // still offline: the rest wait
    }
  } finally {
    flushing = false;
  }
}

export default function Rant() {
  const ui = useUI();
  const online = useOnline();
  useTonight(); // the Bits tab's "N to sort" badge, from launch (lib/tonight)
  const tp = useApi<{ topics: Topic[] }>('/api/topics');
  const rs = useApi<{ rants: RantRow[] }>('/api/rants');
  const takes = useRecordings() as Take[];
  const topics = tp.data?.topics || [];
  const refresh = () => Promise.all([tp.reload(), rs.reload()]);
  useFocusEffect(useCallback(() => { void refresh(); }, [])); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => onTopicsChanged(() => void tp.reload()), []); // eslint-disable-line react-hooks/exhaustive-deps
  // a count the server had not looked for yet comes a moment later (it looks three topics at a time)
  const waiting = topics.some((t) => t.found == null && !t.rant_id);
  useEffect(() => {
    if (!waiting) return;
    const t = setTimeout(() => void tp.reload(), FOUND_RETRY_MS);
    return () => clearTimeout(t);
  }, [waiting, tp.data]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (online && topics.length) void flushScripts(topics).then(() => tp.reload()); }, [online, tp.data == null]); // eslint-disable-line react-hooks/exhaustive-deps

  const fail = (what: string, e: any) => ui.snack(e instanceof ApiError ? `${what}: ${e.message}` : 'Can’t reach Punchable. Nothing changed.');
  // Delete is undoable (HIG/undo-and-redo): the topic is a tombstone, Undo brings it back with its material
  const delTopic = async (t: Topic) => {
    try {
      await rant.deleteTopic(t.id);
      scripts.forget(t.id);
      haptics.warning();
      void tp.reload();
      ui.snack('Topic deleted. Its bits are untouched.', async () => { await rant.undelete(t.id); await tp.reload(); });
    } catch (e: any) { fail('Couldn’t delete it', e); }
  };
  const rename = async (t: Topic) => {
    const text = await askText('Rename the topic', t.text);
    if (!text || text === t.text) return;
    try { await rant.rename(t.id, text); void tp.reload(); } catch (e: any) { fail('Couldn’t rename it', e); }
  };

  const first = leftOff(topics);
  const rest = topics.filter((t) => t !== first);
  const loose = rantsWithoutTopic(rs.data?.rants || [], topics);
  const filmed = filmedList(takes, rs.data?.rants || [], topics);
  const loading = !tp.data && !tp.rawError;
  const nothing = !!tp.data && !topics.length && !loose.length;

  return (
    <Screen title="Rant" testID="rant" largeTitle={native} onRefresh={refresh}
      right={native ? undefined : <View style={{ flexDirection: 'row', alignItems: 'center' }}>
        <IconBtn testID="rant-new" label="New topic" onPress={newTopic}><Icon name="plus" size={24} color={C.amber} sw={2.4} /></IconBtn>
        <Avatar />
      </View>}>
      {native && (
        // HIG/toolbars: the primary action trailing, the face last
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button icon="plus" accessibilityLabel="New topic" accessibilityHint="Opens a blank topic to name" onPress={newTopic}>New topic</Stack.Toolbar.Button>
          <Stack.Toolbar.View><Avatar ask={false} /></Stack.Toolbar.View>
        </Stack.Toolbar>
      )}

      {first && (
        <View testID="rant-leftoff" style={{ borderRadius: 18, padding: 16, gap: 8, backgroundColor: C.card }}>
          <Pressable accessibilityRole="button" accessibilityHint="Opens the topic" onPress={() => openTopic(first)} style={{ gap: 6 }}>
            <Text accessibilityRole="header" maxFontSizeMultiplier={1.8} style={{ fontFamily: F.g6, fontSize: 20, lineHeight: 26, color: C.text }}>{first.text}</Text>
            {(first.last_lines || []).map((l, i) => (
              <Text key={i} maxFontSizeMultiplier={PROSE_CAP} style={{ fontFamily: F.body, fontSize: 17, lineHeight: 24, color: C.text }}>{l}</Text>
            ))}
            <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>
              {[`Rant v${first.v}`, monthDay(first.at), runOf(first.words)].filter(Boolean).join(' · ')}
            </Text>
          </Pressable>
          <View style={{ flexDirection: 'row', gap: 8, alignItems: 'center' }}>
            <View style={{ flex: 1 }}>
              <Btn v="primary" testID="rant-keep-writing" label="Keep writing" onPress={() => router.push({ pathname: '/topic/[topic]/script', params: { topic: first.id } } as any)} />
            </View>
            <Btn testID="rant-film" label="Film it" icon={<Icon name="video" size={18} color={C.text} />} onPress={() => router.push(`/film/${enc(first.rant_id as string)}` as any)} />
          </View>
        </View>
      )}

      {loading && <Skeleton lines={4} height={20} testID="rant-skeleton" />}
      {nothing && (
        <Empty text="What do you want to rant about?">
          <Btn v="primary" testID="rant-empty-new" label="New topic" onPress={newTopic} />
        </Empty>
      )}

      {(rest.length > 0 || loose.length > 0) && <SectionHead title="Topics" testID="rant-topics-head" />}
      <List>
        {rest.map((t) => (
          <SwipeRow key={t.id} testID={`topic-swipe-${t.id}`} trailing={{ label: 'Delete', destructive: true, onPress: () => void delTopic(t), testID: `topic-delete-${t.id}` }}>
            {/* HIG/accessibility: the swipe has a menu twin (long press: Rename…, Delete last) */}
            <RowMenu href={`/topic/${enc(t.id)}`} items={[
              { label: 'Rename…', icon: 'pencil', onPress: () => void rename(t), testID: `topic-menu-rename-${t.id}` },
              { label: 'Delete', icon: 'trash', destructive: true, onPress: () => void delTopic(t), testID: `topic-menu-delete-${t.id}` },
            ]}>
              <ListRow testID={`topic-${t.id}`} title={<T v="semi" numberOfLines={2}>{t.text}</T>}
                sub={topicSub(t, takesOf(takes, t.rant_id)[0]?.recorded_at) || undefined} onPress={() => openTopic(t)} />
            </RowMenu>
          </SwipeRow>
        ))}
        {loose.map((r) => (
          <ListRow key={r.id} testID={`rant-loose-${r.id}`} title={<T v="semi" numberOfLines={1}>{r.short_name || r.title}</T>} sub={`Rant v${r.v}`}
            onPress={() => router.push(`/joke/${enc(r.id)}` as any)} />
        ))}
      </List>

      {filmed.length > 0 && (
        <List style={{ marginTop: 8 }}>
          <ListRow testID="rant-filmed" title={<T>Filmed</T>} right={<T v="mut">{String(filmed.length)}</T>} onPress={() => router.push('/filmed' as any)} />
        </List>
      )}
      {tp.rawError && !tp.data ? <T v="mut" testID="rant-error">Can’t reach Punchable: {tp.rawError}</T> : null}
    </Screen>
  );
}
