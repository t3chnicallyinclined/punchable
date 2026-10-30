// The authorization code -> tokens, exactly once per result (login.tsx; settings.tsx "Reconnect Google").
// Keyed on the result ONLY, with a ref of the last result handled: expo-auth-session rebuilds `request` when discovery
// lands or the config changes, and the effect this replaces was keyed on [result, request], so a late rebuild
// re-exchanged the same code and Keycloak answered invalid_grant after a successful sign-in (docs/ARCHITECTURE.md
// §7.3). The request (its PKCE verifier) and the callbacks are read at exchange time through a ref kept current by an
// effect, so the hook never writes a ref during render.
import { useEffect, useRef } from 'react';
import type { AuthRequest, AuthSessionResult } from 'expo-auth-session';
import { completeSignIn } from './auth';

export function useCodeExchange(
  result: AuthSessionResult | null,
  request: AuthRequest | null,
  onDone: () => void,
  onError: (e: any) => void,
): void {
  const handled = useRef<AuthSessionResult | null>(null);
  const latest = useRef({ request, onDone, onError });
  useEffect(() => {
    latest.current = { request, onDone, onError };
  });
  useEffect(() => {
    if (result?.type !== 'success' || !result.params.code || handled.current === result) return;
    handled.current = result;
    const { request: req, onDone: done, onError: fail } = latest.current;
    completeSignIn(result.params.code, req?.codeVerifier).then(done, fail);
  }, [result]);
}
