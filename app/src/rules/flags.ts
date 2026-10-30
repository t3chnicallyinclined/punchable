// Remote switches (no React Native imports: tests/flags.test.mjs runs this in Node). The API's /api/health may carry
// a `flags` object (api-dev/server.py, from the PK_APP_FLAGS env), so a feature can be turned off on the phone
// without a build: `{"flags":{"glass_accessory":false}}`. Missing or unreadable = the default (on).
export type Flags = {
  /** The input bar as the tab bar's Liquid Glass bottom accessory (lib/TabsShell.native). Off = the dock above the tab bar. */
  glassAccessory: boolean;
};

export const DEFAULT_FLAGS: Flags = { glassAccessory: true };

/** "0", "off", "false", "no", false, 0 -> false; "1", "on", "true", "yes", true, 1 -> true; anything else -> the default. */
export function asBool(v: unknown, dflt: boolean): boolean {
  if (typeof v === 'boolean') return v;
  if (typeof v === 'number') return v !== 0;
  if (typeof v === 'string') {
    const s = v.trim().toLowerCase();
    if (s === '0' || s === 'off' || s === 'false' || s === 'no') return false;
    if (s === '1' || s === 'on' || s === 'true' || s === 'yes') return true;
  }
  return dflt;
}

/** The /api/health body (or any object) -> flags. A `flags` object wins; a top-level key of the same name is accepted too. */
export function readFlags(body: unknown, base: Flags = DEFAULT_FLAGS): Flags {
  if (!body || typeof body !== 'object') return { ...base };
  const b = body as Record<string, unknown>;
  const f = b.flags && typeof b.flags === 'object' ? (b.flags as Record<string, unknown>) : {};
  const pick = (k: string) => (k in f ? f[k] : b[k]);
  return { glassAccessory: asBool(pick('glass_accessory'), base.glassAccessory) };
}

/** Build-time switch: EXPO_PUBLIC_GLASS_ACCESSORY=0 turns the accessory off before any network. */
export function flagsFromEnv(env: Record<string, string | undefined>): Flags {
  return { glassAccessory: asBool(env.EXPO_PUBLIC_GLASS_ACCESSORY, DEFAULT_FLAGS.glassAccessory) };
}
