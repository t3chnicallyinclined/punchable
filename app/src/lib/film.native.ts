// Film it on the phone (docs/design/SPEC-2026-11-03-film-it.md on the spike's v1 path, SPIKE-2026-11-03 §7.1): the
// native pieces the camera screen (app/film/[id]) and the take player (app/take/[rec]) need, kept out of the route
// files so the web never bundles them (film.ts is the web twin).
//   - expo-camera 58.0.14 and expo-video 58.0.13 are loaded LAZILY: a binary without them (an older dev client, a
//     TestFlight build before this one) sees "unavailable" on the screen, never a crash at launch (the precedent is
//     lib/transcribe.native.ts; every root-level route is required at startup, SPIKE §9a).
//   - the take: recordAsync writes <cache>/Camera/<uuid>.mov; the cache can be purged, so the file moves into
//     Documents/recordings the moment the take stops (stashTake), under a generated name the recognizer can open
//     (rules/memoRules safeFileName). Redo deletes it (discardTake).
//   - Keep (keepTake): the one copy goes to Photos with ADD-ONLY access (expo-media-library requestPermissionsAsync(true):
//     NSPhotoLibraryAddUsageDescription, the smaller grant, HIG/privacy "request access only to data that you actually
//     need"; no album in v1, SPIKE §6). The call is saveToLibraryAsync (UISaveVideoAtPathToSavedPhotosAlbum, no
//     readback): the module's createAssetAsync fetches the asset back after writing it (MediaLibraryUtilities.swift:262
//     getAssetBy), which an add-only grant cannot do (INFERRED), and the add-only requester reports `accessPrivileges:
//     "all"` for that grant too (Requesters.swift:33-40), so there is no honest way to tell full access apart here
//     [review]: v1 keeps no asset id. Then the row joins the recordings queue as kind `film`: transcribed here, the
//     .srt written beside it, the words sent as an idea on the bit, the Documents .mov deleted once the words exist
//     (lib/recordings.native afterFilmWords).
//   - the audio session: expo-camera's capture session configures the app's shared AVAudioSession itself and does not
//     restore it (SPIKE §1; Apple: "the audio session's original state isn't restored"); harmless for the set recorder
//     and the memo player, which set their modes on every start. The REVIEW player (expo-video) keeps whatever category
//     the capture left (expo-video/ios/AudioSessionPolicy.swift keeps a playAndRecord session as is), so before it is
//     made the mode becomes playback (playbackMode: the memo player's exact call) and the take plays through the
//     speaker; Redo flips the camera's `active` back on and the capture reconfigures for recording by itself [review].
//     lib/audioBusy says "recording" for the whole life of the camera screen so the transcription queue never runs a
//     file through the recognizer while the camera encodes, and no memo plays over the preview.
import { setAudioModeAsync } from 'expo-audio';
import * as FS from 'expo-file-system/legacy';
import * as ML from 'expo-media-library/legacy';
import type { CutShort, LocalRec } from '@/rules/memoRules';
import { isSafeFileName, safeFileName } from '@/rules/memoRules';
import { VIDEO_CODEC } from '@/rules/filmRules';
import { addRecording } from './recordings';

export const filmSupported = true;
export type CameraModule = typeof import('expo-camera');
export type VideoModule = typeof import('expo-video');

let camP: Promise<CameraModule | null> | null = null;
/** expo-camera, or null when this binary does not have it. */
export function loadCamera(): Promise<CameraModule | null> {
  if (!camP) camP = import('expo-camera').then((m) => (m?.CameraView ? m : null)).catch(() => null);
  return camP;
}
let vidP: Promise<VideoModule | null> | null = null;
/** expo-video, or null when this binary does not have it. */
export function loadVideo(): Promise<VideoModule | null> {
  if (!vidP) vidP = import('expo-video').then((m) => (m?.VideoView ? m : null)).catch(() => null);
  return vidP;
}

export type Access = { camera: boolean; mic: boolean };
/** The camera, then the microphone (already granted for sets and memos; asked here if not). Asked on the first Film
 *  it, never at launch (HIG/privacy). */
export async function requestCameraAccess(cam: CameraModule): Promise<Access> {
  let camera = false;
  let mic = false;
  try { camera = (await cam.Camera.requestCameraPermissionsAsync()).granted; } catch { camera = false; }
  try { mic = (await cam.Camera.requestMicrophonePermissionsAsync()).granted; } catch { mic = false; }
  return { camera, mic };
}
/** Free space on the phone in bytes; null when it cannot be read (the disc is not greyed for a reading we lack). */
export async function freeBytes(): Promise<number | null> {
  try {
    const n = await FS.getFreeDiskStorageAsync();
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}
/** HEVC when this phone offers it (SPIKE §9a: half the size of H.264 at the same quality), else the camera's default. */
export async function takeCodec(cam: CameraModule): Promise<'hvc1' | undefined> {
  try {
    const list = await cam.CameraView.getAvailableVideoCodecsAsync();
    return list.includes(VIDEO_CODEC) ? VIDEO_CODEC : undefined;
  } catch {
    return undefined;
  }
}

export type Take = { uri: string; name: string; durationMs: number; recordedAt: string; cutShort: CutShort | null };
const dir = () => `${FS.documentDirectory}recordings/`;
/** The take just stopped: out of the cache and into Documents/recordings at once, under a safe name. */
export async function stashTake(cacheUri: string, durationMs: number, cutShort: CutShort | null): Promise<Take> {
  await FS.makeDirectoryAsync(dir(), { intermediates: true }).catch(() => {});
  const ext = (cacheUri.split('.').pop() || 'mov').toLowerCase();
  const name = safeFileName(new Date().toISOString(), Math.random().toString(36).slice(2), isSafeFileName(ext) ? ext : 'mov');
  const uri = dir() + name;
  await FS.moveAsync({ from: cacheUri, to: uri });
  return { uri, name, durationMs, recordedAt: new Date(Date.now() - durationMs).toISOString(), cutShort };
}
/** Redo, or Discard behind Close: the file goes; nothing else exists yet. */
export async function discardTake(t: Take | null | undefined): Promise<void> {
  if (!t) return;
  await FS.deleteAsync(t.uri, { idempotent: true }).catch(() => {});
}

export type PhotosAdd = 'granted' | 'denied';
/** Add-only Photos access, asked on the first Keep (not when Film it opens). */
export async function requestPhotosAdd(): Promise<PhotosAdd> {
  try {
    const p = await ML.requestPermissionsAsync(true);
    return p.granted ? 'granted' : 'denied';
  } catch {
    return 'denied';
  }
}
/** The review is about to play: the playback category (the speaker, the silent switch honoured), the memo player's
 *  exact call. Never while a take runs. */
export async function playbackMode(): Promise<void> {
  try {
    await setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true, shouldPlayInBackground: false, interruptionMode: 'doNotMix' });
  } catch {}
}
/** Keep: the one copy into Photos, then the row into the recordings queue (kind `film`). Throws when Photos refused
 *  the file; the take is still in Documents and the screen says so. */
export async function keepTake(t: Take, meta: { bit: { id: string; title: string }; script: string }): Promise<LocalRec> {
  if ((await requestPhotosAdd()) !== 'granted') throw new Error('no Photos');
  await ML.saveToLibraryAsync(t.uri);
  return addRecording({ uri: t.uri, name: t.name, type: 'video/quicktime', durationMs: t.durationMs }, {
    kind: 'film', recordedAt: t.recordedAt, bit: meta.bit, cutShort: t.cutShort, assetId: null, script: meta.script,
  });
}
