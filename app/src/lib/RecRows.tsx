// The rows for recordings kept on this iPhone (voice memos and taped sets; SPEC 2026-10-31 §6, §7). One component for
// Bits › Memos, a bit's page and Sets › Recorded. A row: his first words (a memo) or the set's title, the date ·
// length · where the words are (rules/memoRules statusWords), a 44-pt play/pause on the trailing side. Tap a filed
// row → the bit; a failed one → Try again; one that needs permission → Settings. The same verbs live on a swipe,
// the long-press context menu (HIG.md: menus keep a visible path) and VoiceOver's actions. Delete cannot be undone,
// so it confirms with an action sheet (HIG/action-sheets: destructive first) that says where the words stay.
// A filmed take (kind `film`, Film it SPEC 2026-11-03 §2.7) is a row on the bit's On tape screen: "Filmed · Nov 3 ·
// 0:48" · "112 words"; its play glyph opens the take full-screen (/take/[rec]) while the .mov is still on the phone,
// Photos once the one copy lives there; "Share captions…" hands the .srt to the share sheet; Delete is the words'.
import { Linking, Platform, Pressable, Share, View } from 'react-native';
import { router } from 'expo-router';
import { actionSheet } from './actionSheet';
import { haptics } from './haptics';
import { Icon } from './Icon';
import { playLocal, useLocalPlaying } from './localPlay';
import { deleteRec, notThisBit, retryRec, srtUriOf, uriOf } from './recordings';
import type { LocalRec } from './recordings';
import { RowMenu, rowActions } from './RowMenu';
import { SwipeRow } from './SwipeRow';
import { C } from './theme';
import { IconBtn, List, T, useUI } from './ui';
import type { MenuItem } from './ui';
import { isRecording } from './audioBusy';
import { clockMs, fmtDate } from '@/rules/format';
import { takeRow } from '@/rules/filmRules';
import { deleteMessage, recTitle, statusWords } from '@/rules/memoRules';

const enc = encodeURIComponent;

/** Where the row's words are on the server, if anywhere he can open. */
function openTarget(r: LocalRec): string | null {
  if (r.state === 'filed' && r.filed_on) return `/joke/${enc(r.filed_on.id)}`;
  if (r.state === 'new_idea' && r.idea_id) return `/joke/${enc(r.idea_id)}`;
  return null;
}
/** Try again helps: a failure that may pass, Dictation turned on since, or a no-speech row he wants run again (menu only).
 *  Not the language failure: it would fail the same way. */
const canRetry = (r: LocalRec) => (r.state === 'failed' && r.error !== 'language-not-supported') || r.state === 'unavailable' || r.state === 'no_speech';
/** What a tap on the row's text does (a no-speech row's action is play, on the button; Try again stays in its menu). */
const tapsTo = (r: LocalRec, target: string | null) => (target ? 'open' : r.state === 'needs_permission' ? 'settings' : canRetry(r) && r.state !== 'no_speech' ? 'retry' : null);

function Row({ r }: { r: LocalRec }) {
  const ui = useUI();
  const p = useLocalPlaying();
  const key = `rec:${r.id}`;
  const on = p.key === key && p.playing;
  const target = openTarget(r);
  const film = r.kind === 'film';
  const kindWord = r.kind === 'memo' ? 'memo' : film ? 'take' : 'set';
  const inPhotos = film && !!r.video_gone;
  const srt = film ? srtUriOf(r) : null;

  const openPhotos = () => void Linking.openURL('photos-redirect://').catch(() => ui.snack('Open Photos to find it.'));
  const play = () => {
    if (film) return inPhotos ? openPhotos() : router.push(`/take/${enc(r.id)}` as any);
    if (isRecording()) return ui.snack('Stop the recording first.');
    playLocal(uriOf(r), key).catch((e) => ui.snack(e?.message || 'Couldn’t play it'));
  };
  const del = () => actionSheet({
    title: film ? 'Delete the words?' : 'Delete the recording?', message: deleteMessage(r),
    options: [
      { label: film ? 'Delete the words' : 'Delete the recording', destructive: true, onPress: () => { haptics.warning(); void deleteRec(r.id); } },
      { label: 'Cancel', cancel: true },
    ],
  });
  const shareCaptions = () => { if (srt) Share.share({ url: srt, title: 'Captions' }).catch(() => ui.snack('Sharing is not available here')); };
  const detach = async () => {
    try {
      await notThisBit(r.id);
      ui.snack('Saved as a new idea instead');
    } catch (e: any) {
      ui.snack(`Couldn’t move it: ${e?.message || 'no answer'}`);
    }
  };
  const does = tapsTo(r, target);
  const tap = () => {
    if (does === 'open') return router.push(target as any);
    if (does === 'settings') return void Linking.openSettings();
    if (does === 'retry') return retryRec(r.id);
  };

  const items: MenuItem[] = [
    film ? { label: inPhotos ? 'Open Photos' : 'Play', icon: inPhotos ? 'photo.on.rectangle' : 'play.fill', onPress: play } : { label: on ? 'Pause' : 'Play', icon: on ? 'pause.fill' : 'play.fill', onPress: play },
    ...(srt ? [{ label: 'Share captions…', icon: 'square.and.arrow.up', onPress: shareCaptions }] : []),
    ...(target && !film ? [{ label: r.state === 'filed' ? 'Open the bit' : 'Open the idea', icon: 'doc.text', onPress: () => router.push(target as any) }] : []),
    ...(r.state === 'filed' && r.merge_id && !film ? [{ label: 'Not this bit', icon: 'arrow.uturn.backward', onPress: detach }] : []),
    ...(canRetry(r) ? [{ label: 'Try again', icon: 'arrow.clockwise', onPress: () => retryRec(r.id) }] : []),
    ...(r.state === 'needs_permission' ? [{ label: 'Open Settings', icon: 'gear', onPress: () => void Linking.openSettings() }] : []),
    { label: film ? 'Delete the words' : 'Delete', icon: 'trash', destructive: true, onPress: del },
  ];
  const take = film ? takeRow(r, statusWords(r)) : null;
  const sub = take ? take.sub : [fmtDate(r.recorded_at), r.duration_s > 0 ? clockMs(r.duration_s * 1000) : null, statusWords(r)].filter(Boolean).join(' · ');
  const title = take ? take.label : recTitle(r);
  const label = take ? [take.label.replace(/ · /g, ', '), take.sub].filter(Boolean).join(', ')
    : [r.kind === 'memo' ? 'Memo' : 'Set', title === 'Set' || title === 'Memo' ? null : title, fmtDate(r.recorded_at), r.duration_s > 0 ? clockMs(r.duration_s * 1000) : null, statusWords(r)].filter(Boolean).join(', ');
  const hint = does === 'open' ? (r.state === 'filed' ? `Opens ${r.filed_on?.title || 'the bit'}` : 'Opens the idea') : does === 'settings' ? 'Opens Settings' : does === 'retry' ? 'Transcribes it again' : undefined;

  return (
    <SwipeRow testID={`rec-swipe-${r.id}`} a11yPassThrough
      trailing={{ label: film ? 'Delete the words' : 'Delete', destructive: true, color: C.red, onPress: del, testID: `rec-delete-${r.id}` }}
      leading={r.state === 'filed' && r.merge_id && !film ? { label: 'Not this bit', color: C.tabOff, onPress: detach, testID: `rec-notthis-${r.id}` } : null}
      onLongPress={Platform.OS === 'ios' ? undefined : () => actionSheet({ title, options: [...items.map((it) => ({ label: it.label, onPress: it.onPress, destructive: it.destructive })), { label: 'Cancel', cancel: true }] })}>
      {/* the play button is a sibling of the row's text, so VoiceOver reaches both (an accessible parent hides its children) */}
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 56 }}>
        <RowMenu href={target || '/bits'} items={items}>
          <Pressable testID={`rec-${r.id}`} accessible accessibilityRole={does ? 'button' : undefined} accessibilityLabel={label} accessibilityHint={hint}
            {...rowActions(items)} onPress={does ? tap : undefined} style={{ flex: 1, minWidth: 0, paddingVertical: 10 }}>
            <T v="semi" numberOfLines={3}>{title}</T>
            <T v="mut">{sub}</T>
          </Pressable>
        </RowMenu>
        <IconBtn testID={`rec-play-${r.id}`} label={inPhotos ? 'Open Photos' : on ? 'Pause' : `Play ${kindWord}`} onPress={play}>
          <Icon name={inPhotos ? 'photos' : on ? 'pause' : 'play'} size={20} color={C.amber} />
        </IconBtn>
      </View>
    </SwipeRow>
  );
}

/** A list of this phone's recordings, newest first. */
export function RecRows({ recs, testID }: { recs: LocalRec[]; testID?: string }) {
  const rows = [...recs].sort((a, b) => b.recorded_at.localeCompare(a.recorded_at));
  return (
    <List>
      {rows.map((r) => <View key={r.id} testID={testID ? `${testID}-${r.id}` : undefined}><Row r={r} /></View>)}
    </List>
  );
}
