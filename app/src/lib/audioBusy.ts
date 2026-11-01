// Whether a recording is running on this phone (the recorder sets it). Two readers: the transcription queue does not
// run while a tape runs (no transcribing on stage, SPEC 2026-10-31 §7), and a memo does not play over a recording
// (playback switches the audio mode, which would end the take). Plain module, one boolean, no platform split.
let recording = false;
const listeners = new Set<(on: boolean) => void>();

export function setRecording(on: boolean) {
  if (on === recording) return;
  recording = on;
  listeners.forEach((f) => f(on));
}
export const isRecording = () => recording;
export function onRecording(f: (on: boolean) => void): () => void {
  listeners.add(f);
  return () => {
    listeners.delete(f);
  };
}
