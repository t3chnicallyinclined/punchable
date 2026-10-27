// Web: no Photos library. Import is a file picker (several files at once) + a YouTube link; the same grouping applies.
import type { Clip } from './importgroup';

export const supportsLibrary = false;
export type Access = 'granted' | 'limited' | 'denied' | 'undetermined';
export async function access(): Promise<Access> {
  return 'denied';
}
export async function requestAccess(): Promise<Access> {
  return 'denied';
}
export async function shareMore() {}
export async function recentVideos(_days = 14, _minS = 0): Promise<Clip[]> {
  return [];
}
export async function blobOf(c: Clip): Promise<Blob> {
  if (!c.file) throw new Error('no file');
  return c.file;
}

function durationOf(f: File): Promise<number> {
  return new Promise((res) => {
    try {
      const url = URL.createObjectURL(f);
      const el = document.createElement(f.type.startsWith('audio') ? 'audio' : 'video');
      el.preload = 'metadata';
      const done = (d: number) => {
        URL.revokeObjectURL(url);
        res(isFinite(d) ? d : 0);
      };
      el.onloadedmetadata = () => done(el.duration);
      el.onerror = () => done(0);
      setTimeout(() => done(0), 4000);
      el.src = url;
    } catch {
      res(0);
    }
  });
}

/** <input type=file multiple>: each file becomes a clip, timed by its lastModified, so same-night grouping still works. */
export function pickVideos(): Promise<Clip[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.multiple = true;
    input.accept = 'video/*,audio/*';
    input.style.display = 'none';
    input.setAttribute('data-testid', 'import-file-input');
    input.onchange = async () => {
      const files = [...(input.files || [])];
      input.remove();
      const clips = await Promise.all(files.map(async (f, i) => ({
        id: `file:${f.name}:${f.size}:${i}`, filename: f.name, creationTime: f.lastModified || Date.now(), duration: await durationOf(f), file: f,
      })));
      resolve(clips);
    };
    document.body.appendChild(input);
    input.click();
  });
}
