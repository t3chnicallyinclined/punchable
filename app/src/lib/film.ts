// Web: no camera path (Film it is in the iPhone app; the bit's More menu does not offer it here). This twin keeps
// expo-camera and expo-video out of the web bundle; the route files render one line when reached by URL.
import type { CutShort, LocalRec } from '@/rules/memoRules';

export const filmSupported = false;
export type CameraModule = typeof import('expo-camera');
export type VideoModule = typeof import('expo-video');
export async function loadCamera(): Promise<CameraModule | null> {
  return null;
}
export async function loadVideo(): Promise<VideoModule | null> {
  return null;
}
export type Access = { camera: boolean; mic: boolean };
export async function requestCameraAccess(_cam: CameraModule): Promise<Access> {
  return { camera: false, mic: false };
}
export async function freeBytes(): Promise<number | null> {
  return null;
}
export async function takeCodec(_cam: CameraModule): Promise<'hvc1' | undefined> {
  return undefined;
}
export type Take = { uri: string; name: string; durationMs: number; recordedAt: string; cutShort: CutShort | null };
export async function stashTake(_cacheUri: string, _durationMs: number, _cutShort: CutShort | null): Promise<Take> {
  throw new Error('Film it is in the iPhone app.');
}
export async function discardTake(_t: Take | null | undefined): Promise<void> {}
export type PhotosAdd = 'granted' | 'denied';
export async function requestPhotosAdd(): Promise<PhotosAdd> {
  return 'denied';
}
export async function playbackMode(): Promise<void> {}
export async function keepTake(_t: Take, _meta: { bit: { id: string; title: string }; script: string }): Promise<LocalRec> {
  throw new Error('Film it is in the iPhone app.');
}
