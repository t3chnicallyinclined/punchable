// Plays a recording kept on this phone (a voice memo or a taped set, Documents/recordings; SPEC 2026-10-31 §6). One
// player at a time over expo-audio's createAudioPlayer (not a hook, so it is released by hand: remove() on stop, on
// the end of the file and before the next one). Before playing, the audio mode becomes playback (allowsRecording
// false: the playback category, so it plays through the speaker, not the earpiece; no background play); the
// recorder sets its recording mode again on every start (lib/recorder.native). HIG/playing-audio: a note-taking app
// with a recording mode "might switch its category to playback if it lets people play the recorded notes".
// Never while a recording runs (switching the mode would end the take).
import { useEffect, useState } from 'react';
import { createAudioPlayer, setAudioModeAsync } from 'expo-audio';
import type { AudioPlayer, AudioStatus } from 'expo-audio';
import { isRecording } from './audioBusy';

export const localPlaySupported = true;
type PlayState = { key: string | null; playing: boolean; at: number };
let player: AudioPlayer | null = null;
let gen = 0;   // each play request's number: a tap that lost the race to a newer one never makes a player [review]
let state: PlayState = { key: null, playing: false, at: 0 };
const listeners = new Set<(s: PlayState) => void>();
const emit = (s: PlayState) => {
  state = s;
  listeners.forEach((f) => f(s));
};

export function stopLocal() {
  const p = player;
  player = null;
  if (p) {
    try { p.pause(); } catch {}
    try { p.remove(); } catch {}
  }
  emit({ key: null, playing: false, at: 0 });
}

/** Play (or pause, if it is the one playing) the recording at `uri` (a file:// URI under Documents/recordings). */
export async function playLocal(uri: string, key: string) {
  if (state.key === key && player) {
    if (state.playing) {
      player.pause();
      emit({ ...state, playing: false });
    } else {
      player.play();
      emit({ ...state, playing: true });
    }
    return;
  }
  if (isRecording()) throw new Error('Stop the recording first.');
  const my = ++gen;
  stopLocal();
  await setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true, shouldPlayInBackground: false, interruptionMode: 'doNotMix' });
  if (my !== gen) return;                     // another row was tapped while the mode switched
  if (isRecording()) throw new Error('Stop the recording first.');
  stopLocal();
  const p = createAudioPlayer({ uri });
  player = p;
  p.addListener('playbackStatusUpdate', (s: AudioStatus) => {
    if (player !== p) return;
    if (s.didJustFinish) return stopLocal();
    emit({ key, playing: s.playing, at: Math.round((s.currentTime || 0) * 1000) });
  });
  p.play();
  emit({ key, playing: true, at: 0 });
}

export function useLocalPlaying(): PlayState {
  const [s, setS] = useState(state);
  useEffect(() => {
    listeners.add(setS);
    return () => {
      listeners.delete(setS);
    };
  }, []);
  return s;
}
