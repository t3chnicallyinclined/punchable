// "▶ Hear it": plays a span of a set's audio, seeking into the file.
import { useEffect, useState } from 'react';
import { Linking, Pressable, Text, View } from 'react-native';
import { onPlayState, playSegment } from './media';
import { C, fmtMs } from './theme';
import { T, s, useUI } from './ui';

function usePlaying() {
  const [p, setP] = useState<{ playing: boolean; key: string | null; at: number }>({ playing: false, key: null, at: 0 });
  useEffect(() => onPlayState(setP), []);
  return p;
}

/** Where a recording lives when Punchable keeps no copy of it (online-first): YouTube, his Drive, or his phone. */
export type Reference = { kind: 'youtube' | 'drive' | 'file'; url?: string; name?: string | null; video_id?: string };

export function youtubeAt(url: string, ms: number) {
  return `${url}${url.includes('?') ? '&' : '?'}t=${Math.max(0, Math.floor(ms / 1000))}s`;
}

export function HearIt({ audio, play, k, sub, testID, reference }: { audio: string | null; play: { start_ms: number; end_ms: number }; k: string; sub: string; testID?: string; reference?: Reference | null }) {
  const ui = useUI();
  const p = usePlaying();
  const on = p.playing && p.key === k;
  if (!audio && reference && reference.url) {
    const url = reference.kind === 'youtube' ? youtubeAt(reference.url, play.start_ms) : reference.url;
    return (
      <Pressable testID={testID} accessibilityRole="link" onPress={() => Linking.openURL(url)} style={[s.inner, s.row, { minHeight: 56 }]}>
        <View style={{ width: 40, height: 40, borderRadius: 20, backgroundColor: C.amber, alignItems: 'center', justifyContent: 'center' }}>
          <Text style={{ color: C.bg, fontSize: 16 }}>▶</Text>
        </View>
        <View style={{ flex: 1 }}>
          <T>{reference.kind === 'youtube' ? `Watch on YouTube at ${fmtMs(play.start_ms)}` : 'Open the recording in Google Drive'}</T>
          <T v="small">{reference.kind === 'youtube' ? sub : reference.name || 'your voice memo'}</T>
        </View>
      </Pressable>
    );
  }
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={on ? 'Pause' : 'Hear it'} disabled={!audio}
      onPress={() => audio && playSegment(audio, play.start_ms, play.end_ms, k).catch((e) => ui.snack(e.message))}
      style={[s.inner, s.row, { minHeight: 56, opacity: audio ? 1 : 0.5 }]}>
      <View style={{ width: 40, height: 40, borderRadius: 20, backgroundColor: C.amber, alignItems: 'center', justifyContent: 'center' }}>
        <Text style={{ color: C.bg, fontSize: 16 }}>{on ? '❚❚' : '▶'}</Text>
      </View>
      <View style={{ flex: 1 }}>
        <T>{on ? `Playing · ${fmtMs(p.at)}` : 'Hear it'}</T>
        <T v="small">{audio ? sub : reference?.kind === 'file' ? `On your phone: ${reference.name || 'the recording'}` : 'No audio for this set'}</T>
      </View>
    </Pressable>
  );
}

