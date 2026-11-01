// Web: no recordings on this device (memos and taped sets live on the iPhone: localPlay.native.ts plays them there).
export const localPlaySupported = false;
type PlayState = { key: string | null; playing: boolean; at: number };
const idle: PlayState = { key: null, playing: false, at: 0 };

export function stopLocal() {}
export async function playLocal(_uri: string, _key: string) {
  throw new Error('Recordings play in the iPhone app.');
}
export function useLocalPlaying(): PlayState {
  return idle;
}
