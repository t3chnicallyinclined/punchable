// Import (UX_SPEC 4.8): which of my recent videos are sets, and which show were they.
// iPhone/Android: Photos (last 14 days, ≥ 3:00 by default, same-night grouping, GPS → venue). Web: pick files
// (several at once; same grouping by time) or paste a YouTube link. Uploads go through POST /api/upload (inbox).
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Image, Platform, Pressable, Switch, Text, TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { ApiError, post, upload, useApi } from '@/lib/api';
import { GeoJSON, creative, useCreative } from '@/lib/creative';
import { Clip, SET_MIN_S, ShowGroup, alignDefault, clipTime, groupShows, splitGroup } from '@/lib/importgroup';
import { fmtDur } from '@/lib/laughs';
import * as Photos from '@/lib/photos';
import { C, F } from '@/lib/theme';
import { AIBadge, Btn, Card, Empty, Icon, Screen, T, TextBtn, TopBar, s, useUI } from '@/lib/ui';

type Status = Record<string, 'queued' | 'uploading' | 'done' | 'failed'>;

export default function Import() {
  const ui = useUI();
  const [access, setAccess] = useState<Photos.Access | 'web' | null>(Photos.supportsLibrary ? null : 'web');
  const [explained, setExplained] = useState(!Photos.supportsLibrary);
  const [clips, setClips] = useState<Clip[]>([]);
  const [showShort, setShowShort] = useState(false);
  const [split, setSplit] = useState<string[]>([]);
  const [align, setAlign] = useState<boolean | null>(null);
  const [status, setStatus] = useState<Status>({});
  const [yt, setYt] = useState('');
  const g = useCreative<GeoJSON>('venues.geojson', creative.geojson);
  const inbox = useApi<{ items: any[] }>('/api/inbox');
  const venues = useMemo(() => (g.data?.features || []).filter((f) => f.geometry).map((f) => ({ venue_id: f.properties.venue_id, name: f.properties.name, lat: f.geometry!.coordinates[1], lng: f.geometry!.coordinates[0] })), [g.data]);
  const importedNames = useMemo(() => new Set((inbox.data?.items || []).map((x) => String(x.title || '').toLowerCase())), [inbox.data]);

  const loadLibrary = useCallback(async () => {
    const a = await Photos.access();
    setAccess(a);
    if (a === 'granted' || a === 'limited') {
      const v = await Photos.recentVideos(14, SET_MIN_S);
      setClips(v.map((c) => ({ ...c, imported: importedNames.has(c.filename.toLowerCase()) })));
    }
  }, [importedNames]);
  useEffect(() => {
    if (Photos.supportsLibrary && explained) loadLibrary();
  }, [explained, loadLibrary]);

  const long = clips.filter((c) => c.duration >= SET_MIN_S || c.file);
  const short = clips.filter((c) => !(c.duration >= SET_MIN_S || c.file));
  const visible = showShort ? clips : long;
  const groups = useMemo(() => {
    let gs = groupShows(visible.filter((c) => !c.imported), venues);
    for (const k of split) gs = splitGroup(gs, k);
    return gs;
  }, [visible, venues, split]);
  const target = ui.workingSet?.target_s ?? null;
  const alignOn = align ?? (groups[0] ? alignDefault(groups[0].total_s, target) : false);
  const nClips = groups.reduce((a, x) => a + x.clips.length, 0);
  const bytes = groups.reduce((a, x) => a + x.clips.reduce((b, c) => b + ((c.file as any)?.size || 0), 0), 0);

  const doImport = async () => {
    let ok = 0;
    for (const gr of groups) {
      const showName = `${gr.venue?.name || 'Show'} ${new Date(gr.start).toISOString().slice(0, 10)}`;
      for (const c of gr.clips) {
        setStatus((s2) => ({ ...s2, [c.id]: 'uploading' }));
        try {
          const blob = await Photos.blobOf(c);
          await upload(blob, {
            kind: 'set', filename: c.filename, title: showName, durationS: c.duration, recordedAt: new Date(c.creationTime).toISOString(),
            venue: gr.venue?.name, lat: c.lat ?? undefined, lng: c.lng ?? undefined, showGroup: gr.key, alignWorkingSet: alignOn && ui.workingSet ? ui.workingSet.id : undefined,
          });
          setStatus((s2) => ({ ...s2, [c.id]: 'done' }));
          ok++;
        } catch {
          setStatus((s2) => ({ ...s2, [c.id]: 'failed' }));
        }
      }
    }
    const failed = nClips - ok;
    ui.snack(failed ? `${failed} ${failed === 1 ? 'clip' : 'clips'} wait in the outbox · Retry` : `Imported ${groups.length} ${groups.length === 1 ? 'show' : 'shows'} · processing`);
    if (!failed) router.navigate('/' as any);
  };

  const pick = async () => {
    const got = await Photos.pickVideos();
    if (got.length) setClips((x) => [...x, ...got]);
  };
  const youtube = async () => {
    try {
      const r = await post<{ waiting_for?: string | null; note?: string | null }>('/api/import/youtube', { url: yt.trim() });
      ui.snack(r?.waiting_for === 'transcriber' ? `Queued. ${r.note || 'Transcription happens on your phone for now; this link is waiting.'}` : 'Queued: the server will fetch the audio');
      setYt('');
    } catch (e: any) {
      ui.snack(e instanceof ApiError && e.status === 404 ? 'YouTube import isn’t on the server yet (requested). Download the video and pick the file instead.' : `Couldn't queue it: ${e.message}`);
    }
  };

  const tile = (c: Clip) => (
    <View key={c.id} testID={`clip-${c.filename}`} style={{ width: '48%', borderRadius: 10, borderWidth: 1, borderColor: C.border, backgroundColor: C.surface2, overflow: 'hidden', opacity: c.imported ? 0.45 : 1 }}>
      <View style={{ height: 84, backgroundColor: C.bgDeep, alignItems: 'center', justifyContent: 'center' }}>
        {c.uri && Platform.OS !== 'web' ? <Image source={{ uri: c.uri }} style={{ width: '100%', height: '100%' }} /> : <Icon name="photos" size={28} color={C.mut} />}
        {!c.imported && <View style={{ position: 'absolute', top: 8, right: 8, width: 10, height: 10, borderRadius: 5, backgroundColor: C.amber }} />}
      </View>
      <View style={{ padding: 8, gap: 2 }}>
        <T v="small" style={{ color: C.text }}>{c.imported ? 'Imported' : clipTime(c.creationTime)}</T>
        <T v="small">{c.duration ? fmtDur(c.duration) : 'length unknown'}{status[c.id] ? ` · ${status[c.id]}` : ''}</T>
      </View>
    </View>
  );

  const groupBlock = (gr: ShowGroup) => (
    <View key={gr.key} testID={`group-${gr.key}`} style={{ gap: 8 }}>
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', justifyContent: 'space-between', rowGap: 8 }}>{gr.clips.map(tile)}</View>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
        <T v="small" testID={`group-line-${gr.key}`} style={{ color: C.text2 }}>↳ {gr.clips.length > 1 ? `grouped: one show · ${gr.clips.length} clips` : 'one show'} · {gr.total_s ? fmtDur(gr.total_s) : ''}</T>
        {gr.venue && <><T v="small" style={{ color: C.text2 }}>· {gr.venue.name}? </T><AIBadge /><T v="small">· {gr.venue.distance_m} m away</T></>}
        {gr.clips.length > 1 && <TextBtn testID={`split-${gr.key}`} label="Split" onPress={() => setSplit((x) => [...x, gr.key])} />}
      </View>
    </View>
  );

  const dock = (
    <View style={{ gap: 8 }}>
      {ui.workingSet && (
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10, minHeight: 44 }}>
          <Switch testID="align-toggle" value={alignOn} onValueChange={setAlign} trackColor={{ true: C.amber, false: C.surface2 }} accessibilityLabel="Line up with the Working set" />
          <T style={{ flex: 1, fontSize: 15 }}>Line up with {ui.workingSet.title}</T>
        </View>
      )}
      <T v="small">{Photos.supportsLibrary ? `Audio only leaves your phone${bytes ? ` (${Math.round(bytes / 1e6)} MB)` : ''}` : `The files upload to your server${bytes ? ` (${Math.round(bytes / 1e6)} MB)` : ''}`}</T>
      <Btn testID="import-go" label={groups.length ? `Import ${groups.length} ${groups.length === 1 ? 'show' : 'shows'} (${nClips} ${nClips === 1 ? 'clip' : 'clips'})` : 'Import'} v="primary" disabled={!groups.length} onPress={doImport} />
    </View>
  );

  return (
    <Screen title="Import sets" task testID="import" header={<TopBar title="Import sets" task left={<TextBtn label="Cancel" onPress={() => (router.canGoBack() ? router.back() : router.navigate('/' as any))} />} />} dock={dock}>
      {Photos.supportsLibrary && !explained && (
        <Card testID="photos-explainer" style={{ gap: 10 }}>
          <T v="h2">Find your sets in Photos</T>
          <T>Punchable looks at your recent videos to find sets. Only the audio of the ones you pick leaves your phone.</T>
          <Btn label="Continue" v="primary" onPress={async () => { await Photos.requestAccess(); setExplained(true); }} />
        </Card>
      )}
      {access === 'denied' && Photos.supportsLibrary && explained && <Card><T>Photos access is off. Turn it on in Settings › Punchable › Photos, or pick files.</T></Card>}
      {access === 'limited' && <Card style={{ gap: 6 }}><T>You’ve shared {clips.length} videos.</T><Btn small label="Share more" onPress={async () => { await Photos.shareMore(); loadLibrary(); }} /></Card>}
      {access === 'web' && (
        <Card testID="drop-zone" style={{ gap: 10, borderStyle: 'dashed', borderColor: C.borderStrong }}>
          <T v="h2">Pick set videos</T>
          <T v="mut">Choose one or more files. Clips from the same night are grouped into one show.</T>
          <Btn testID="pick-files" label="Choose files" v="add" onPress={pick} />
          <T v="small">or paste a YouTube link</T>
          <View style={{ flexDirection: 'row', gap: 8 }}>
            <TextInput testID="youtube-link" value={yt} onChangeText={setYt} placeholder="https://youtu.be/…" placeholderTextColor={C.dim} autoCapitalize="none"
              style={{ flex: 1, minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
            <Btn small label="Add" disabled={!/^https?:\/\/(www\.)?(youtube\.com|youtu\.be)\//.test(yt.trim())} onPress={youtube} />
          </View>
        </Card>
      )}
      {(access === 'granted' || access === 'limited' || clips.length > 0) && (
        <View style={{ gap: 14 }}>
          {Photos.supportsLibrary && <T v="eb">Last 14 days · videos 3 min+</T>}
          {!groups.length && !clips.length && Photos.supportsLibrary && <Empty text="No set-length videos in the last 2 weeks."><Btn small label="Show all videos" onPress={() => setShowShort(true)} /></Empty>}
          {groups.map(groupBlock)}
          {clips.filter((c) => c.imported).length > 0 && <View style={{ flexDirection: 'row', flexWrap: 'wrap', justifyContent: 'space-between', rowGap: 8 }}>{clips.filter((c) => c.imported).map(tile)}</View>}
          {!!short.length && !showShort && <TextBtn testID="show-short" label={`${short.length} short clips hidden · Show`} color={C.mut} onPress={() => setShowShort(true)} />}
        </View>
      )}
    </Screen>
  );
}
