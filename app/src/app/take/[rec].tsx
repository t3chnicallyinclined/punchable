// A filmed take, played (Film it, SPEC 2026-11-03 §2.7): the On tape screen's play glyph opens the take full-screen in
// the system player (expo-video, nativeControls, the original aspect: HIG/playing-video). While the take is still in
// Documents (its words on their way) it plays here; once the words exist the .mov has left Documents and the one copy
// is in Photos, so this screen offers Photos instead (the row says "in Photos"). Web: one line.
import React, { useEffect, useMemo, useState } from 'react';
import { Linking, Platform, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { useRecordings, uriOf } from '@/lib/recordings';
import * as Film from '@/lib/film';
import type { VideoModule } from '@/lib/film';
import { C } from '@/lib/theme';
import { Btn, T, TextBtn, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';
import { takeRow } from '@/rules/filmRules';
import { statusWords } from '@/rules/memoRules';

const native = Platform.OS !== 'web';

export default function TakeScreen() {
  const p = useLocalSearchParams<{ rec: string }>();
  const recId = decodeURIComponent(String(p.rec || ''));
  const ui = useUI();
  const recs = useRecordings();
  const r = useMemo(() => recs.find((x) => x.id === recId && x.kind === 'film') || null, [recs, recId]);
  const [vid, setVid] = useState<VideoModule | null | undefined>(undefined);
  // the native player is held in state (render reads it, so it is not a ref); it is made and released by the effect
  const [player, setPlayer] = useState<any>(null);
  const uri = r && !r.video_gone ? uriOf(r) : null;
  const leave = () => (router.canGoBack() ? router.back() : router.navigate('/bits' as any));

  useEffect(() => {
    if (!native) return;
    let alive = true;
    void Film.loadVideo().then((m) => alive && setVid(m));
    return () => { alive = false; };
  }, []);
  useEffect(() => {
    if (!vid || !uri) return;
    let pl: any = null;
    try {
      pl = vid.createVideoPlayer(uri);
      pl.play();
    } catch {
      pl = null;
    }
    // a native resource made here and released below: the state follows it (an external system, the effect's job)
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setPlayer(pl);
    return () => {
      setPlayer(null);
      try { pl?.release?.(); } catch {}
    };
  }, [vid, uri]);

  const title = r ? takeRow(r, statusWords(r)).label : 'Filmed';
  const VideoView = vid?.VideoView;
  return (
    <Screen title={title} task deep scroll={false} testID="take"
      header={<TopBar task title={title} left={<TextBtn testID="take-close" label="Close" onPress={leave} />} right={<View style={{ width: 66 }} />} />}>
      <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', padding: 16, gap: 12 }}>
        {!native || !Film.filmSupported ? (
          <T v="mut" style={{ textAlign: 'center' }}>Filmed takes play in the iPhone app.</T>
        ) : !r ? (
          <T v="mut">This take is gone.</T>
        ) : !uri ? (
          <>
            <T v="mut" testID="take-in-photos">In Photos</T>
            <Btn testID="take-open-photos" label="Open Photos" onPress={() => void Linking.openURL('photos-redirect://').catch(() => ui.snack('Open Photos to find it.'))} />
          </>
        ) : vid === null ? (
          <T v="mut">Not in this build yet.</T>
        ) : VideoView && player ? (
          <View style={{ height: '100%', aspectRatio: 9 / 16, maxWidth: '100%', borderRadius: 16, overflow: 'hidden', backgroundColor: C.surface2 }}>
            <VideoView player={player} style={{ flex: 1 }} nativeControls contentFit="contain" />
          </View>
        ) : (
          <T v="mut">Opening…</T>
        )}
      </View>
    </Screen>
  );
}
