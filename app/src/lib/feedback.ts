// Haptic meaning (HIG/playing-haptics), kept pure so the order can be unit-tested without a device:
//  - Success "indicates that a task or action has completed": it plays AFTER the write, and only when the write did
//    something (not when a picker opened instead, not on an error);
//  - Warning "indicates that a task or action has produced a warning": opening a confirmation sheet is not one;
//  - "Don't overuse haptics": one light impact when a hold starts, none when it ends.

export type Haptic = 'selection' | 'light' | 'medium' | 'success' | 'warning' | 'error';
export type HapticPlayer = { [K in Haptic]: () => void };

/** Run the write; play success only after it resolved and `done` says it wrote. Errors pass through untouched. */
export async function afterWrite<T>(write: () => Promise<T>, done: (r: T) => boolean, h: Pick<HapticPlayer, 'success'>): Promise<T> {
  const r = await write();
  if (done(r)) h.success();
  return r;
}

/** Hold to talk: one impact on press-in, none on release. */
export function holdHaptic(phase: 'in' | 'out'): Haptic | null {
  return phase === 'in' ? 'light' : null;
}

/** Tapping Stop opens a confirmation: no haptic (the outcome plays its own: success on save). */
export function stopHaptic(): Haptic | null {
  return null;
}
