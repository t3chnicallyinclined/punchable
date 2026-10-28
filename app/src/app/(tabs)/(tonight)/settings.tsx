// Settings (UX_SPEC 4.13): Connections first (one row per outside source: a plain status and one action), then who
// sees my material and what I control. Saved through PATCH /api/settings; GET /api/connections is read-only.
import React, { useEffect, useRef, useState } from 'react';
import { Linking, Platform, Pressable, Switch, TextInput, View } from 'react-native';
import * as AuthSession from 'expo-auth-session';
import { router } from 'expo-router';
import { ApiError, del as apiDelete, get, patch, useApi } from '@/lib/api';
import { AUTH_ENABLED, authRequestConfig, completeSignIn, getDiscovery, rememberWay } from '@/lib/auth';
import { creative } from '@/lib/creative';
import { haptics } from '@/lib/haptics';
import { getPrefs, onPrefs, setPrefs, SchemePref } from '@/lib/prefs';
import * as Photos from '@/lib/photos';
import { C, F } from '@/lib/theme';
import { Btn, Card, Dot, List, ListRow, Seg, Sheet, SheetHead, T, s, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const SITE = 'https://punchable.io';

/** Appearance and haptics are THIS device's preferences (lib/prefs), never his material. */
function DeviceCard() {
  const [p, setP] = useState(getPrefs());
  useEffect(() => onPrefs(setP), []);
  return (
    <Card testID="device-prefs" style={{ gap: 10 }}>
      <T v="eb">This device</T>
      <View style={{ gap: 6 }}>
        <T v="semi">Appearance</T>
        <Seg<SchemePref> testID="appearance" items={[['system', 'System'], ['dark', 'Dark'], ['light', 'Light']]} value={p.appearance} onChange={(v) => { haptics.selection(); setPrefs({ appearance: v }); }} />
        <T v="small">Dark is the default. System follows your phone or browser.</T>
      </View>
      {haptics.available && (
        <View style={[s.row, { minHeight: 56 }]}>
          <View style={{ flex: 1 }}><T v="semi">Haptics</T><T v="mut">A tap you can feel on reorder, save and remove.</T></View>
          <Switch testID="toggle-haptics" value={p.haptics} onValueChange={(b) => setPrefs({ haptics: b })} trackColor={{ true: C.amber, false: C.surface2 }} accessibilityLabel="Haptics" />
        </View>
      )}
    </Card>
  );
}

type Connections = {
  google_docs: {
    status: 'connected' | 'not_set_up' | 'not_synced_yet' | 'stale' | 'error' | 'api_down' | 'not_reported';
    last_sync: string | null; last_ok: string | null; docs_linked: number; joke_docs: number; boards: number; conflicts: number; errors_last_run: number;
  };
  youtube: { sets: number };
  ai: { provider: string; worker: { online: boolean; last_seen: string | null; name: string | null; where?: string; hosted?: boolean; model?: string | null; paused?: string | null } };
  transcriber?: { where: string; waiting: number; note: string | null };
  audio_storage: { status: string; provider: string };
};

/** "3 min ago" from an ISO time (his clock; rounded, never a false-precise number). */
function ago(iso: string | null | undefined): string {
  if (!iso) return '';
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return '';
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  const d = Math.round(s / 86400);
  return d === 1 ? 'yesterday' : `${d} days ago`;
}
const n = (x: number) => x.toLocaleString('en-US');
type Tone = 'ok' | 'warn' | 'off';
const TONE: Record<Tone, string> = { ok: C.green, warn: C.redSoft, off: C.dim };

function googleLine(g: Connections['google_docs'] | undefined): [string, Tone] {
  if (!g) return ['Checking…', 'off'];
  const docs = g.docs_linked ? ` · ${n(g.docs_linked)} docs linked` : '';
  const conf = g.conflicts ? ` · ${g.conflicts} edit${g.conflicts === 1 ? '' : 's'} not written back` : '';
  switch (g.status) {
    case 'connected': return [`Connected · synced ${ago(g.last_sync)}${docs}${conf}`, 'ok'];
    case 'not_synced_yet': return ['Set up · first sync hasn’t run yet', 'off'];
    case 'stale': return [`Last synced ${ago(g.last_sync)}. Sync has not run in a day.${docs}`, 'warn'];
    case 'error': return [`Last sync ${ago(g.last_sync)} had ${g.errors_last_run} problem${g.errors_last_run === 1 ? '' : 's'}${g.last_ok ? ` · last good sync ${ago(g.last_ok)}` : ''}`, 'warn'];
    case 'api_down': return [`Your PC’s sync couldn’t reach Punchable ${ago(g.last_sync)}${g.last_ok ? ` · last good sync ${ago(g.last_ok)}` : ''}`, 'warn'];
    case 'not_reported': return ['This server hasn’t heard from a sync yet.', 'off'];
    default: return ['Not connected', 'off'];
  }
}

function ConnRow({ name, line, tone, action, testID, note }: { name: string; line: string; tone: Tone; action?: React.ReactNode; testID: string; note?: React.ReactNode }) {
  return (
    <View testID={testID} style={{ flexDirection: 'row', alignItems: 'center', gap: 12, minHeight: 64, paddingVertical: 10 }}>
      <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <View style={{ flexShrink: 0 }}><Dot color={TONE[tone]} hollow={tone === 'off'} /></View>
          <T v="semi">{name}</T>
        </View>
        <T v="mut" testID={`${testID}-status`}>{line}</T>
        {note}
      </View>
      {action}
    </View>
  );
}

/** Settings > Connections. Each outside source: what it is, a plain status, one action. */
function ConnectionsCard() {
  const ui = useUI();
  const c = useApi<Connections>('/api/connections', { cache: false });
  const [discovery, setDiscovery] = useState<AuthSession.DiscoveryDocument | null>(null);
  useEffect(() => {
    if (AUTH_ENABLED) getDiscovery().then(setDiscovery, () => {}); // local copy: no sign-in server to ask
  }, []);
  // "Reconnect Google": the same sign-in as the login screen's Google button, but prompt=login makes Keycloak go to
  // Google again even with a live session, so Google grants Drive access afresh (realm IdP: offline + consent) and
  // Keycloak stores the new token that gsync reads.
  const [req, res, prompt] = AuthSession.useAuthRequest(authRequestConfig({ kc_idp_hint: 'google', prompt: 'login' }), discovery);
  useEffect(() => {
    if (res?.type === 'success' && res.params.code) {
      completeSignIn(res.params.code, req?.codeVerifier)
        .then(() => {
          rememberWay('google');
          ui.snack('Google reconnected. Your PC’s sync uses it on its next run if the old access had stopped, otherwise within a day.');
          c.reload();
        })
        .catch((e) => ui.snack(`Google sign-in didn’t finish: ${e?.message || 'try again'}`));
    } else if (res?.type === 'error') ui.snack('Google sign-in didn’t finish. Try again.');
  }, [res]); // eslint-disable-line react-hooks/exhaustive-deps

  const d = c.data;
  const [gLine, gTone] = c.error && !d ? ['Can’t reach Punchable to check', 'warn' as Tone] : googleLine(d?.google_docs);
  const w = d?.ai.worker;
  const aiLine = !d ? 'Checking…' : w?.online && w.paused ? `Paused · ${w.paused}` : w?.online ? `Online · checked in ${ago(w.last_seen)}` : w?.last_seen ? `Offline · last seen ${ago(w.last_seen)}. Drafts wait until it’s back.` : 'Offline · not seen since Punchable started. Drafts wait until it’s on.';
  const aiWhere = w?.model ? `${w.model}` : w?.hosted || w?.where === 'the server' ? 'The writer, on the server' : 'The writer';
  const tr = d?.transcriber;
  const photosLine = Photos.supportsLibrary ? 'Recent set videos from your Photos. Only the audio is uploaded.' : 'On the phone app it reads recent set videos from Photos. Here: pick video files.';
  return (
    <Card testID="connections" style={{ gap: 0 }}>
      <T v="eb">Connections</T>
      <List>
        <ConnRow testID="src-gdocs" name="Google Docs" line={gLine} tone={gTone}
          note={!AUTH_ENABLED ? <T v="small" testID="src-gdocs-why">Reconnect works on punchable.io. Sign-in is off on this local copy.</T> : undefined}
          action={<Btn small testID="src-gdocs-reconnect" label="Reconnect Google" disabled={!AUTH_ENABLED || !req}
            accessibilityLabel={AUTH_ENABLED ? 'Reconnect Google' : 'Reconnect Google: sign-in is off on this local copy'}
            onPress={() => prompt().catch((e) => ui.snack(/popup/i.test(String(e?.message)) ? 'Your browser blocked the Google window. Allow pop-ups for this site and try again.' : 'Google sign-in didn’t start. Try again.'))} />} />
        <ConnRow testID="src-youtube" name="YouTube" line={d ? (d.youtube.sets ? `${n(d.youtube.sets)} sets imported by link` : 'No sets yet · paste a video link') : 'Checking…'} tone={d?.youtube.sets ? 'ok' : 'off'}
          action={<Btn small testID="src-youtube-import" label="Import a link" onPress={() => router.push('/import' as any)} />} />
        <ConnRow testID="src-photos" name="Phone videos" line={photosLine} tone="off"
          action={<Btn small testID="src-photos-import" label="Import videos" onPress={() => router.push('/import' as any)} />} />
        <ConnRow testID="src-ai" name="AI for drafts" line={`${aiWhere} · ${aiLine}`} tone={!d ? 'off' : w?.online && !w.paused ? 'ok' : 'warn'}
          action={<Btn small testID="src-ai-check" label="Check again" onPress={() => c.reload()} />} />
        <ConnRow testID="src-transcriber" name="Transcription" line={tr ? (tr.waiting ? `On your phone (coming) · ${tr.waiting} file${tr.waiting === 1 ? '' : 's'} waiting for a transcript` : 'On your phone (coming) · nothing waiting') : 'On your phone (coming)'} tone={tr?.waiting ? 'warn' : 'off'} />
        <ConnRow testID="src-audio" name="Audio storage" line="Cloudflare R2 · not set up yet. No recording is kept on the server." tone="off" />
      </List>
    </Card>
  );
}

type Settings = { provider: 'hosted' | 'chatgpt' | 'key' | 'custom'; training: boolean; upload_video: boolean; cellular: boolean };
const DEFAULTS: Settings = { provider: 'hosted', training: false, upload_video: false, cellular: false };
const KEY = 'pk.settings.v1';
const PROVIDERS: { k: Settings['provider']; title: string; sub: string }[] = [
  { k: 'hosted', title: 'Punchable hosted', sub: 'Claude Opus 5.5 · sees the text of the bit you’re working on' },
  { k: 'chatgpt', title: 'Sign in with ChatGPT', sub: 'Your ChatGPT account · sees the text of the bit you’re working on' },
  { k: 'key', title: 'Your API key', sub: 'Billed to you · sees the text of the bit you’re working on' },
  { k: 'custom', title: 'Custom endpoint', sub: 'Any OpenAI-compatible server you run' },
];
const ROUTES: [string, string][] = [
  ['Audio of your sets and notes', 'Your server: transcribed and laughs measured there. Video stays on your phone.'],
  ['Text for writing tasks (drafts, bridges, Ask)', 'The AI provider below, only the bit you’re working on.'],
  ['Map tiles', 'OpenStreetMap on the web, Apple Maps on iPhone: they see the area you look at, never your sets.'],
  ['Google Docs', 'Reads your docs to keep jokes in sync; writes only to the Punchable folder and docs it created. Court documents are never opened.'],
  ['Your decisions and boards', 'Your server, append-only. Nobody else reads them.'],
];

export default function SettingsPage() {
  const ui = useUI();
  const remote = useApi<Partial<Settings>>('/api/settings', { cache: false });
  const [st, setSt] = useState<Settings>(() => {
    try { return { ...DEFAULTS, ...JSON.parse((globalThis as any).localStorage?.getItem(KEY) || '{}') }; } catch { return DEFAULTS; }
  });
  const [local, setLocal] = useState(false);
  const [del, setDel] = useState(false);
  const [typed, setTyped] = useState('');
  const touched = useRef<Set<keyof Settings>>(new Set());
  useEffect(() => {
    // the server's answer can arrive after he has already tapped something: never undo his tap
    if (remote.data) setSt((x) => {
      const merged = { ...x, ...remote.data };
      touched.current.forEach((k) => { (merged as any)[k] = (x as any)[k]; });
      return merged;
    });
  }, [remote.data]);
  const save = async (p: Partial<Settings>) => {
    (Object.keys(p) as (keyof Settings)[]).forEach((k) => touched.current.add(k));
    const next = { ...st, ...p };
    setSt(next);
    try { (globalThis as any).localStorage?.setItem(KEY, JSON.stringify(next)); } catch {}
    try {
      await patch('/api/settings', p);
      setLocal(false);
    } catch (e) {
      setLocal(e instanceof ApiError && (e.status === 404 || e.status === 405));
    }
  };

  const exportAll = async () => {
    try {
      const [decisions, sets, boards] = await Promise.all([
        get<any>('/api/decisions?limit=100000'), get<any>('/api/sets'), creative.boards().catch(() => null),
      ]);
      const analyses = await Promise.all((sets.sets || []).map((x: any) => get<any>(`/api/sets/${encodeURIComponent(x.id)}/analysis`).catch(() => null)));
      const blob = new Blob([JSON.stringify({ exported_at: new Date().toISOString(), decisions, sets: sets.sets, analyses, boards }, null, 1)], { type: 'application/json' });
      if (Platform.OS === 'web') {
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = `punchable-export-${new Date().toISOString().slice(0, 10)}.json`;
        a.click();
        ui.snack('Export downloaded (decisions, sets, analyses, boards)');
      } else ui.snack('Export on the phone build is coming next');
    } catch (e: any) {
      ui.snack(`Couldn't export: ${e.message}`);
    }
  };

  const toggle = (testID: string, label: string, sub: string, v: boolean, on: (b: boolean) => void) => (
    <View style={[s.row, { minHeight: 64, paddingVertical: 8 }]}>
      <View style={{ flex: 1 }}><T v="semi">{label}</T><T v="mut">{sub}</T></View>
      <Switch testID={testID} value={v} onValueChange={on} trackColor={{ true: C.amber, false: C.surface2 }} accessibilityLabel={label} />
    </View>
  );

  return (
    <Screen title="Settings" back testID="settings">
      <T v="h1">Settings & privacy</T>
      {local && <T v="small" testID="settings-local">Saved on this device. The server doesn’t store settings yet.</T>}
      <DeviceCard />
      <ConnectionsCard />
      <Card testID="where-material-goes" style={{ gap: 6 }}>
        <T v="eb">Where your material goes</T>
        {ROUTES.map(([what, where]) => (
          <View key={what} style={{ paddingVertical: 6, borderTopWidth: 1, borderTopColor: C.border, gap: 2 }}>
            <T v="semi" style={{ fontSize: 15 }}>{what}</T>
            <T v="mut">{where}</T>
          </View>
        ))}
      </Card>
      <Card testID="provider" style={{ gap: 8 }}>
        <T v="eb">AI provider for writing tasks</T>
        {PROVIDERS.map((p) => (
          <Pressable key={p.k} testID={`provider-${p.k}`} accessibilityRole="radio" accessibilityState={{ checked: st.provider === p.k }} aria-checked={st.provider === p.k} onPress={() => save({ provider: p.k })}
            style={[s.inner, { flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 56 }, st.provider === p.k && { borderColor: C.amber }]}>
            <View style={{ width: 20, height: 20, borderRadius: 10, borderWidth: 2, borderColor: st.provider === p.k ? C.amber : C.borderStrong, alignItems: 'center', justifyContent: 'center' }}>
              {st.provider === p.k && <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: C.amber }} />}
            </View>
            <View style={{ flex: 1 }}><T v="semi">{p.title}</T><T v="small">{p.sub}</T></View>
          </Pressable>
        ))}
        <T v="small">Each provider’s score on your checked examples shows here once there are enough of them.</T>
      </Card>
      <Card style={{ gap: 0 }}>
        <T v="eb">Controls</T>
        {toggle('toggle-training', 'Let my material train models', 'Off, and stays off unless you turn it on', st.training, (b) => save({ training: b }))}
        {toggle('toggle-video', 'Upload video', 'Off: only the audio leaves your phone', st.upload_video, (b) => save({ upload_video: b }))}
        {toggle('toggle-cellular', 'Use cellular for uploads', 'Off: imports wait for Wi-Fi in the outbox', st.cellular, (b) => save({ cellular: b }))}
      </Card>
      <Card style={{ gap: 4 }}>
        <List>
          <ListRow testID="open-learned" title="What I’ve learned about you" sub="The AI’s read of your taste, and how sure it is" onPress={() => router.push('/learned' as any)} />
          <ListRow testID="audit" title="Show what the AI decided" sub="Your notebook, filtered to things the AI set and you haven’t touched" onPress={() => router.push('/notebook?who=ai' as any)} />
          <ListRow testID="open-privacy" title="Privacy policy" sub="What Punchable stores, where, and your choices" onPress={() => Linking.openURL(`${SITE}/privacy`)} />
          <ListRow testID="open-terms" title="Terms" sub="The short version: it’s yours" onPress={() => Linking.openURL(`${SITE}/terms`)} />
        </List>
      </Card>
      <Card style={{ gap: 10 }}>
        <Btn testID="export-all" label="Export everything" onPress={exportAll} />
        <Btn testID="delete-all" label="Delete my account and data" v="danger" onPress={() => { haptics.warning(); setTyped(''); setDel(true); }} />
        <T v="small">Deletion is a request: nothing is removed until the owner runs it by hand, and you get told when it’s done.</T>
      </Card>
      <Sheet visible={del} onClose={() => setDel(false)} label="Delete my account and data" testID="delete-sheet">
        <SheetHead title="Delete everything?" onCancel={() => setDel(false)} testID="delete" />
        <T v="mut" style={{ marginVertical: 8 }}>Your account, your docs’ text, sets, analyses, decisions and boards are deleted from the server. This can’t be undone. Export first if you want a copy.</T>
        <T v="small">Type “delete my material” to confirm</T>
        <TextInput testID="delete-typed" value={typed} onChangeText={setTyped} autoCapitalize="none" accessibilityLabel="Type delete my material to confirm"
          style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16, marginVertical: 8 }} />
        <Btn testID="delete-confirm" label="Delete everything" v="danger" disabled={typed.trim().toLowerCase() !== 'delete my material'} onPress={async () => {
          setDel(false);
          try {
            const r = await apiDelete<{ status: string; would_delete?: { what: string; count: number | null }[] }>('/api/me', { confirm: 'delete my material' });
            if (r.status === 'requested') ui.snack('Deletion requested. The owner removes it by hand and tells you when it’s done.');
            else if (r.status === 'dry_run') ui.snack(`Local copy: nothing is deleted here. A deletion would cover ${r.would_delete?.length || 0} things.`);
            else ui.snack('Deletion requested.');
          } catch (e: any) {
            ui.snack(`Couldn’t request deletion: ${e?.message || 'try again'}. Nothing was deleted.`);
          }
        }} />
      </Sheet>
    </Screen>
  );
}
