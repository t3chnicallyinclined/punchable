// Audio playback, recording and file picking.
// Web: HTMLAudioElement, MediaRecorder + Web Audio level meter, <input type=file>.
// Native: TODO(native). Use expo-audio (installed, see app.json plugin) for playback
// (useAudioPlayer + seekTo) and recording (useAudioRecorder with RecordingPresets.HIGH_QUALITY,
// background audio mode for locked-screen recording), plus a Live Activity timer.
// Those are stubbed for v0 so the web build stays the focus.
import { Linking, Platform } from 'react-native';
import { API_URL } from './api';

export const isWeb = Platform.OS === 'web';

// ---------------------------------------------------------------- playback
type PlayState = { playing: boolean; key: string | null; at: number };
let el: HTMLAudioElement | null = null;
let stopAt = 0;
let curKey: string | null = null;
const listeners = new Set<(s: PlayState) => void>();
const emit = () => {
  const s = { playing: !!el && !el.paused, key: curKey, at: el ? el.currentTime * 1000 : 0 };
  listeners.forEach((f) => f(s));
};
export const onPlayState = (f: (s: PlayState) => void) => {
  listeners.add(f);
  return () => {
    listeners.delete(f);
  };
};

function audioEl(): HTMLAudioElement {
  if (!el) {
    el = new Audio();
    el.preload = 'auto';
    el.addEventListener('timeupdate', () => {
      if (el && stopAt && el.currentTime * 1000 >= stopAt) {
        el.pause();
        stopAt = 0;
      }
      emit();
    });
    el.addEventListener('pause', emit);
    el.addEventListener('play', emit);
    el.addEventListener('ended', emit);
    (globalThis as any).__punchableAudio = el; // for the browser test
  }
  return el;
}

/** Play [fromMs, toMs] of an API audio path. Seeks into the file (the API serves byte ranges). */
/** A YouTube set keeps no audio (online-first): its "audio" is `youtube:<url>`, and playing it opens the video at the
 * moment (&t=<seconds>s). Every existing play button keeps working through playSegment. */
export function youtubeSource(ref?: { kind: string; url?: string } | null): string | null {
  return ref?.kind === 'youtube' && ref.url ? `youtube:${ref.url}` : null;
}
export function youtubeAt(url: string, ms: number) {
  return `${url}${url.includes('?') ? '&' : '?'}t=${Math.max(0, Math.floor(ms / 1000))}s`;
}

export async function playSegment(path: string, fromMs: number, toMs: number, key: string) {
  if (path.startsWith('youtube:')) {
    await Linking.openURL(youtubeAt(path.slice('youtube:'.length), fromMs));
    return;
  }
  if (!isWeb) {
    // TODO(native): expo-audio useAudioPlayer(source).seekTo(fromMs / 1000).play()
    throw new Error('Playback on the phone build is not wired yet');
  }
  const a = audioEl();
  const src = API_URL + path;
  if (curKey === key && !a.paused) {
    a.pause();
    return;
  }
  curKey = key;
  if (a.src !== src) {
    a.src = src;
    await new Promise<void>((res, rej) => {
      const ok = () => { cleanup(); res(); };
      const bad = () => { cleanup(); rej(new Error('Could not load the audio')); };
      const cleanup = () => { a.removeEventListener('loadedmetadata', ok); a.removeEventListener('error', bad); };
      a.addEventListener('loadedmetadata', ok);
      a.addEventListener('error', bad);
      a.load();
    });
  }
  a.currentTime = fromMs / 1000;
  stopAt = toMs;
  (globalThis as any).__punchableLastSeek = { key, fromMs, toMs, src };
  await a.play();
  emit();
}

export function stopPlayback() {
  if (el) el.pause();
  stopAt = 0;
}

// ---------------------------------------------------------------- recording
export type Recorder = {
  start: () => Promise<void>;
  pause: () => void;
  resume: () => void;
  stop: () => Promise<Blob>;
  cancel: () => void;
  elapsedMs: () => number;
  level: () => number; // 0..1
  state: () => 'idle' | 'recording' | 'paused' | 'stopped';
  mime: string;
};

export function micSupported() {
  return isWeb && typeof navigator !== 'undefined' && !!navigator.mediaDevices?.getUserMedia && typeof (globalThis as any).MediaRecorder !== 'undefined';
}

export function createRecorder(): Recorder {
  if (!isWeb) {
    // TODO(native): expo-audio useAudioRecorder + setAudioModeAsync({ allowsRecording: true, shouldPlayInBackground: true })
    throw new Error('Recording on the phone build is not wired yet');
  }
  let stream: MediaStream | null = null;
  let rec: MediaRecorder | null = null;
  let chunks: BlobPart[] = [];
  let ctx: AudioContext | null = null;
  let analyser: AnalyserNode | null = null;
  let buf: Uint8Array<ArrayBuffer> | null = null;
  let startedAt = 0;
  let pausedTotal = 0;
  let pausedAt = 0;
  let st: 'idle' | 'recording' | 'paused' | 'stopped' = 'idle';
  const MR = (globalThis as any).MediaRecorder as typeof MediaRecorder;
  const mime = ['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4', 'audio/ogg'].find((m) => MR?.isTypeSupported?.(m)) || '';
  const release = () => {
    stream?.getTracks().forEach((t) => t.stop());
    ctx?.close().catch(() => {});
    stream = null;
    ctx = null;
  };
  return {
    mime,
    async start() {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false } as any });
      rec = new MR(stream, mime ? { mimeType: mime } : undefined);
      chunks = [];
      rec.ondataavailable = (e) => e.data && e.data.size && chunks.push(e.data);
      rec.start(1000);
      try {
        const AC = (globalThis as any).AudioContext || (globalThis as any).webkitAudioContext;
        ctx = new AC();
        const srcNode = ctx!.createMediaStreamSource(stream);
        analyser = ctx!.createAnalyser();
        analyser.fftSize = 512;
        buf = new Uint8Array(new ArrayBuffer(analyser.fftSize));
        srcNode.connect(analyser);
      } catch {
        analyser = null;
      }
      startedAt = Date.now();
      pausedTotal = 0;
      st = 'recording';
    },
    pause() {
      if (rec && st === 'recording') {
        rec.pause();
        pausedAt = Date.now();
        st = 'paused';
      }
    },
    resume() {
      if (rec && st === 'paused') {
        rec.resume();
        pausedTotal += Date.now() - pausedAt;
        st = 'recording';
      }
    },
    stop() {
      return new Promise<Blob>((res) => {
        if (!rec) return res(new Blob([], { type: mime }));
        if (st === 'paused') pausedTotal += Date.now() - pausedAt;
        const end = Date.now();
        rec.onstop = () => {
          release();
          st = 'stopped';
          startedAt = end - (end - startedAt); // freeze
          res(new Blob(chunks, { type: (rec?.mimeType || mime || 'audio/webm').split(';')[0] }));
        };
        rec.stop();
      });
    },
    cancel() {
      try {
        rec?.stop();
      } catch {}
      release();
      st = 'stopped';
    },
    elapsedMs() {
      if (st === 'idle') return 0;
      const now = st === 'paused' ? pausedAt : Date.now();
      return Math.max(0, now - startedAt - pausedTotal);
    },
    level() {
      if (!analyser || !buf || st !== 'recording') return 0;
      analyser.getByteTimeDomainData(buf);
      let peak = 0;
      for (let i = 0; i < buf.length; i++) peak = Math.max(peak, Math.abs(buf[i] - 128));
      return Math.min(1, peak / 90);
    },
    state: () => st,
  };
}

/** Ask for the mic once (after our explainer). Resolves true when allowed. */
export async function requestMic(): Promise<boolean> {
  if (!micSupported()) return false;
  try {
    const s = await navigator.mediaDevices.getUserMedia({ audio: true });
    s.getTracks().forEach((t) => t.stop());
    return true;
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------- file picking
export type Picked = { blob: Blob; name: string; lastModified: number | null };

/** Recordings he picks from his phone's Files (Voice Memos saved there) or his computer: one or several. The file
 * goes up to be transcribed and is deleted from the server once processed; Punchable keeps the words and a reference
 * (its name and date), not the recording. Web: the browser's picker; iOS/Android: expo-document-picker. */
export async function pickRecordings(): Promise<Picked[]> {
  const DP = await import('expo-document-picker');
  const r: any = await DP.getDocumentAsync({ type: ['audio/*', 'video/*'], multiple: true, copyToCacheDirectory: true });
  if (!r || r.canceled || !r.assets?.length) return [];
  const out: Picked[] = [];
  for (const a of r.assets as any[]) {
    const blob: Blob = a.file instanceof Blob ? a.file : await (await fetch(a.uri)).blob();
    out.push({ blob, name: a.name || 'recording', lastModified: a.lastModified ?? a.file?.lastModified ?? null });
  }
  return out;
}

export function pickFile(accept = 'audio/*,video/*'): Promise<File | null> {
  if (!isWeb) {
    // TODO(native): expo-document-picker / the iOS share extension for Voice Memos.
    return Promise.resolve(null);
  }
  return new Promise((res) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.style.display = 'none';
    input.setAttribute('data-testid', 'file-input');
    input.onchange = () => {
      const f = input.files?.[0] || null;
      input.remove();
      res(f);
    };
    document.body.appendChild(input);
    input.click();
  });
}

// mic permission explainer: shown once, remembered per browser
const MIC_KEY = 'punchable.micExplained';
export const micExplained = () => {
  try {
    return isWeb && globalThis.localStorage?.getItem(MIC_KEY) === '1';
  } catch {
    return false;
  }
};
export const setMicExplained = () => {
  try {
    globalThis.localStorage?.setItem(MIC_KEY, '1');
  } catch {}
};
