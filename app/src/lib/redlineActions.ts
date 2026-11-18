// What the marks do once he asks for a draft (SPEC 2026-11-16 Slice 3; SPEC 2026-11-17 one notebook): Send, one round,
// in document order, each mark anchored to its words. Used by the page (Get a new draft) and the draft (Try again).
import { ApiError } from './api';
import { haptics } from './haptics';
import { setMarks } from './marks';
import { redline } from './redline';
import * as M from '@/rules/markRules';

type UI = { snack: (msg: string, undo?: () => Promise<void> | void) => void };

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
