// Web: no recordings are kept on this device. Voice memos and taped sets are recorded, kept and transcribed on the
// iPhone (recordings.native.ts); their words reach the web through the server like everything else.
import type { LocalRec, RecKind } from '@/rules/memoRules';
import type { Recording } from './recorderBase';

export type { LocalRec } from '@/rules/memoRules';
export const recordingsSupported = false;
const none: LocalRec[] = [];

export function uriOf(r: Pick<LocalRec, 'file'>): string {
  return r.file;
}
export function startRecordings() {}
export function useRecordings(): LocalRec[] {
  return none;
}
export type AddMeta = { kind: RecKind; recordedAt: string; bit?: { id: string; title: string } | null; board?: { id: string; title: string } | null; title?: string | null; marksMs?: number[] };
export async function addRecording(_rec: Recording, _meta: AddMeta): Promise<LocalRec> {
  throw new Error('Recordings are kept in the iPhone app.');
}
export function retryRec(_id: string) {}
export async function deleteRec(_id: string) {}
export async function notThisBit(_id: string) {}
export async function importFiles(): Promise<number> {
  return 0;
}
