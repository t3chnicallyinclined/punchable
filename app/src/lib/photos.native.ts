// iOS / Android: recent set videos from the Photos library (expo-media-library, legacy API surface in SDK 58).
// Limited access (iOS "selected photos") is handled: we only see what he shared, and "Share more" re-opens the picker.
import * as ML from 'expo-media-library/legacy';
import type { Clip } from './importgroup';

export const supportsLibrary = true;
export type Access = 'granted' | 'limited' | 'denied' | 'undetermined';

export async function access(): Promise<Access> {
  const p = await ML.getPermissionsAsync(false, ['video']);
  if (p.status !== 'granted') return p.canAskAgain === false || p.status === 'denied' ? 'denied' : 'undetermined';
  return p.accessPrivileges === 'limited' ? 'limited' : 'granted';
}
export async function requestAccess(): Promise<Access> {
  await ML.requestPermissionsAsync(false, ['video']);
  return access();
}
export async function shareMore() {
  await ML.presentPermissionsPickerAsync(['video']);
}

/** Videos from the last `days` days, newest first; GPS read for the set-length ones only. */
export async function recentVideos(days = 14, minS = 0): Promise<Clip[]> {
  const page = await ML.getAssetsAsync({
    mediaType: ML.MediaType.video, first: 300, sortBy: [[ML.SortBy.creationTime, false]], createdAfter: Date.now() - days * 86400000,
  });
  const out: Clip[] = [];
  for (const a of page.assets) {
    const c: Clip = { id: a.id, filename: a.filename, creationTime: a.creationTime, duration: a.duration, uri: a.uri };
    if (a.duration >= minS) {
      try {
        const info = await ML.getAssetInfoAsync(a, { shouldDownloadFromNetwork: false });
        c.lat = info.location?.latitude ?? null;
        c.lng = info.location?.longitude ?? null;
        c.uri = info.localUri || a.uri;
      } catch {}
    }
    out.push(c);
  }
  return out;
}

/** The file to upload. TODO(native): extract the audio track on device (AVAssetExportSession / MediaExtractor) so only audio leaves the phone. */
export async function blobOf(c: Clip): Promise<Blob> {
  const r = await fetch(c.uri!);
  return r.blob();
}

/** Web only. */
export async function pickVideos(): Promise<Clip[]> {
  return [];
}
