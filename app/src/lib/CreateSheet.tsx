// The "+" sheet (MixCreate): Bit · Set list · Record a set · Voice memos, and a YouTube link. Every row lands on an
// existing flow: the input bar (/say), a new board (POST /api/boards) opened as a set list, the recorder, the notes
// screen, POST /api/import/youtube (queued; transcription happens on the phone for now, so the link waits).
// `CreateBody` is the content; `CreateSheet` wraps it in the Modal sheet (the web); on the phone the route
// app/create.tsx shows it as a native form sheet whose bar carries Cancel / Back / Start (HIG/sheets).
import React, { useEffect, useState } from 'react';
import { Pressable, TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { post } from './api';
import { creative } from './creative';
import { useBoards } from './boards';
import { C, F } from './theme';
import { Btn, Icon, Sheet, SheetHead, T, useUI } from './ui';
import { fetchWorker } from './worker';

const LENGTHS = [5, 7, 10, 15];
export type CreateMode = 'menu' | 'set';

function Row({ title, sub, icon, onPress, testID }: { title: string; sub: string; icon: string; onPress: () => void; testID: string }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={`${title}. ${sub}`} onPress={onPress}
      style={(st: any) => ({ flexDirection: 'row', alignItems: 'center', gap: 14, minHeight: 64, paddingHorizontal: 14, borderRadius: 16, backgroundColor: st.hovered ? C.card : C.card2 })}>
      <View style={{ width: 40, height: 40, borderRadius: 20, backgroundColor: C.card, alignItems: 'center', justifyContent: 'center' }}><Icon name={icon} size={20} color={C.amber} /></View>
      <View style={{ flex: 1 }}>
        <T v="semi">{title}</T>
        <T v="small">{sub}</T>
      </View>
    </Pressable>
  );
}

/**
 * The sheet's content. `chrome` draws its own title rows (Cancel · New; Back · New set list · Start) for the Modal
 * sheet; the native form sheet owns the bar instead (chrome={false}) and triggers Start through `startRef`.
 */
export function CreateBody({ onClose, mode, setMode, chrome = true, startRef }: {
  onClose: () => void; mode: CreateMode; setMode: (m: CreateMode) => void; chrome?: boolean; startRef?: React.MutableRefObject<() => void>;
}) {
  const ui = useUI();
  const boards = useBoards();
  const [title, setTitle] = useState('');
  const [mins, setMins] = useState(7);
  const [custom, setCustom] = useState('');
  const [yt, setYt] = useState('');
  const [busy, setBusy] = useState(false);

  const makeSet = async () => {
    if (busy) return;
    const m = custom.trim() ? Number(custom) : mins;
    if (!m || m < 1 || m > 240) return ui.snack('Pick a length in minutes (1 to 240)');
    const t = title.trim() || `${new Date().toLocaleDateString(undefined, { weekday: 'long' })} · ${m}`;
    setBusy(true);
    try {
      const v = await creative.createBoard({ title: t, joke_ids: [], target_s: m * 60 });
      boards.setCurrent({ id: v.board.id, title: v.board.title });
      onClose();
      router.push(`/list/${encodeURIComponent(v.board.id)}` as any);
    } catch (e: any) {
      ui.snack(`Couldn't start the set list: ${e.message}`);
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    if (startRef) startRef.current = makeSet;
  });
  const importYt = async () => {
    const u = yt.trim();
    if (!u) return;
    setBusy(true);
    try {
      const r = await post<{ status: string; set?: string; waiting_for?: string | null; note?: string | null }>('/api/import/youtube', { url: u });
      setYt('');
      onClose();
      if (r.status === 'exists' && r.set) { ui.snack('That video is already a set', undefined, { label: 'Open', run: () => router.push(`/set/${encodeURIComponent(r.set!)}` as any) }); }
      else {
        if (r.waiting_for === 'transcriber') ui.snack(`Queued. ${r.note || 'Transcription happens on your phone for now; this link is waiting.'}`);
        else {
          const w = await fetchWorker(0);
          ui.snack(w && !w.online ? 'Queued. The writer is offline: it picks this up when it’s back.' : 'Queued: the server fetches the audio, finds the laughs and the bits. Minutes, not seconds.');
        }
      }
    } catch (e: any) {
      ui.snack(`Couldn't take that link: ${e.message}`);
    } finally {
      setBusy(false);
    }
  };

  if (mode === 'menu') {
    return (
      <View style={{ gap: 10 }}>
        {chrome && <SheetHead title="New" onCancel={onClose} testID="create" />}
        <Row testID="create-bit" title="Bit" sub="Write or say a new one" icon="write" onPress={() => { onClose(); router.push({ pathname: '/say', params: { focus: '1' } } as any); }} />
        <Row testID="create-set" title="Set list" sub="5, 7, 10, 15 or custom" icon="sets" onPress={() => setMode('set')} />
        <Row testID="create-record" title="Record a set" sub="Tape tonight, review after" icon="mic" onPress={() => { onClose(); ui.withMic(() => router.push('/record' as any)); }} />
        <Row testID="create-memos" title="Voice memos" sub="From Files or Voice Memos" icon="upload" onPress={() => { onClose(); router.push('/notes' as any); }} />
        <View style={{ marginTop: 6, gap: 6 }}>
          <T v="small">YouTube link</T>
          <View style={{ flexDirection: 'row', gap: 8 }}>
            {/* HIG/virtual-keyboards: the URL keyboard and a Go return key for a link field; a Clear button while editing */}
            <TextInput testID="create-yt" value={yt} onChangeText={setYt} placeholder="https://youtu.be/…" placeholderTextColor={C.dim} autoCapitalize="none" autoCorrect={false} keyboardType="url"
              returnKeyType="go" onSubmitEditing={importYt} clearButtonMode="while-editing"
              style={{ flex: 1, minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 10, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 15 }} />
            <Btn small testID="create-yt-go" label="Import" disabled={!yt.trim() || busy} onPress={importYt} />
          </View>
          <T v="small">We keep the link and the transcript, never the video.</T>
        </View>
        <T v="small" style={{ textAlign: 'center', marginTop: 4 }}>Or just say it: “new 7 for Friday”</T>
      </View>
    );
  }
  return (
    <View style={{ gap: 10 }}>
      {chrome && <SheetHead title="New set list" onCancel={() => setMode('menu')} cancelLabel="Back" onDone={busy ? undefined : makeSet} doneLabel="Start" testID="create-set" />}
      <TextInput testID="create-set-title" value={title} onChangeText={setTitle} placeholder="Name it (Friday · 7)" placeholderTextColor={C.dim} clearButtonMode="while-editing" returnKeyType="done"
        style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 10, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
      <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
        {LENGTHS.map((m) => <Btn key={m} small testID={`create-len-${m}`} v={mins === m && !custom ? 'primary' : 'plain'} label={`${m} min`} onPress={() => { setMins(m); setCustom(''); }} />)}
        <TextInput testID="create-len-custom" value={custom} onChangeText={setCustom} placeholder="custom" placeholderTextColor={C.dim} keyboardType="number-pad"
          style={{ width: 84, minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 12, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 15 }} />
      </View>
      <Btn testID="create-set-go" v="primary" label={busy ? 'Starting…' : 'Start the set list'} disabled={busy} onPress={makeSet} />
    </View>
  );
}

/** The Modal sheet (the web; also any caller that keeps the in-page sheet). */
export function CreateSheet({ visible, onClose, start = 'menu' }: { visible: boolean; onClose: () => void; start?: CreateMode }) {
  const [mode, setMode] = useState<CreateMode>(start);
  useEffect(() => { if (visible) setMode(start); }, [visible, start]);
  const close = () => { setMode(start); onClose(); };
  return (
    <Sheet visible={visible} onClose={close} label="New" testID="create-sheet">
      {visible && <CreateBody onClose={close} mode={mode} setMode={setMode} />}
    </Sheet>
  );
}
