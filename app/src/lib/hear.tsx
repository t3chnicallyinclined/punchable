// "▶ Hear it": plays a span of a set's audio, seeking into the file.
import { useEffect, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { onPlayState, playSegment } from './media';
import { C, fmtMs } from './theme';
import { T, s, useUI } from './ui';

function usePlaying() {
  const [p, setP] = useState<{ playing: boolean; key: string | null; at: number }>({ playing: false, key: null, at: 0 });
  useEffect(() => onPlayState(setP), []);
  return p;
}

export function HearIt({ audio, play, k, sub, testID }: { audio: string | null; play: { start_ms: number; end_ms: number }; k: string; sub: string; testID?: string }) {
  const ui = useUI();
  const p = usePlaying();
  const on = p.playing && p.key === k;
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={on ? 'Pause' : 'Hear it'} disabled={!audio}
      onPress={() => audio && playSegment(audio, play.start_ms, play.end_ms, k).catch((e) => ui.snack(e.message))}
      style={[s.inner, s.row, { minHeight: 56, opacity: audio ? 1 : 0.5 }]}>
      <View style={{ width: 40, height: 40, borderRadius: 20, backgroundColor: C.amber, alignItems: 'center', justifyContent: 'center' }}>
        <Text style={{ color: C.bg, fontSize: 16 }}>{on ? '❚❚' : '▶'}</Text>
      </View>
      <View style={{ flex: 1 }}>
        <T>{on ? `Playing · ${fmtMs(p.at)}` : 'Hear it'}</T>
        <T v="small">{audio ? sub : 'No audio for this set'}</T>
      </View>
    </Pressable>
  );
}

