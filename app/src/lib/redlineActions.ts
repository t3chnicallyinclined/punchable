// What the marks do once he acts (SPEC 2026-11-16 Slice 3): Send (one round, in document order, each mark anchored to
// its words), and Take it elsewhere (the owner's Redline "Fork": the words go to a set, as the bit's note there, or
// become a new bit; the bit itself is never changed). Used by the bit page and the draft screen.
import { router } from 'expo-router';
import { ApiError, decide, undo } from './api';
import { actionSheet } from './actionSheet';
import { addVersion } from './bitWork';
import { haptics } from './haptics';
import { setMarks } from './marks';
import { redline } from './redline';
import { openSetPicker } from './SetPicker';
import { nameFromFirstLine } from '@/rules/bitWorkRules';
import * as M from '@/rules/markRules';

type UI = { snack: (msg: string, undo?: () => Promise<void> | void, action?: { label: string; run: () => Promise<void> | void }) => void };

/** Send one round. `unit` = the version he marked (round one), `on` = the round whose draft he marked (round two).
 *  The tray is emptied once the server has the round (the round keeps the marks; "Put the marks back" brings them
 *  back if no draft comes). Resolves true when it was sent. */
export async function sendRound(ui: UI, bit: string, where: { unit?: string; on?: string }, text: string, marks: M.Mark[]): Promise<boolean> {
  const b = M.sendBody(text, marks);
  if (b.error) {
    ui.snack(b.error);
    return false;
  }
  try {
    await redline.send(bit, { ...where, marks: b.marks });
  } catch (e: any) {
    if (e instanceof ApiError && e.status === 409) ui.snack(`${e.message}. Your marks are still here.`);
    else ui.snack(e instanceof ApiError ? `Couldn’t send: ${e.message}` : 'Can’t reach Punchable. Your marks are still here.');
    return false;
  }
  haptics.success();
  await setMarks(bit, where.on || null, []);
  if (b.gone.length) ui.snack(`Sent. ${M.markCount(b.gone.length)} on words you have since changed ${b.gone.length === 1 ? 'was' : 'were'} left out.`);
  return true;
}

/** A note on a set is one line of at most 200 characters (pk-api setnotes): the words cut to fit. */
export function asNote(words: string): string {
  const one = words.replace(/\s+/g, ' ').trim();
  return one.length > 200 ? `${one.slice(0, 199)}…` : one;
}

/** Take it elsewhere: an action sheet, then the set picker (the words as the bit's note in the sets he ticks) or a
 *  new bit made of the words. */
export function takeElsewhere(ui: UI, bit: string, shortName: string, words: string) {
  const newBit = async () => {
    try {
      const w = await decide({ target: 'node', action: 'add', after: { type: 'joke', title: nameFromFirstLine(words), stage: 'idea' }, reason: 'a new bit from words marked on another' });
      const id = w.decisions?.[0]?.id;
      if (!id) throw new Error('no id came back');
      const v = await addVersion(id, words.trim(), `from words marked on ${bit}`);
      haptics.success();
      ui.snack('Made a new bit from those words', async () => { await undo([...v.indexes, ...w.indexes]); }, {
        label: 'Open', run: () => router.push(`/joke/${encodeURIComponent(id)}` as any),
      });
    } catch (e: any) {
      ui.snack(e instanceof ApiError ? `Couldn’t make the bit: ${e.message}` : 'Can’t reach Punchable. Nothing changed.');
    }
  };
  actionSheet({
    title: 'Take these words elsewhere',
    message: `“${asNote(words).slice(0, 80)}”`,
    options: [
      { label: 'Add to a set, as a note…', onPress: () => openSetPicker(bit, shortName, asNote(words)) },
      { label: 'New bit from these words', onPress: () => void newBit() },
      { label: 'Cancel', cancel: true },
    ],
  });
}
