// Crash and error reporting: @sentry/react-native pointed at the self-hosted Bugsink (infra/k8s/70-bugsink.yaml;
// infra/OBSERVABILITY.md "Crash reports"). No SaaS account anywhere.
//
//   EXPO_PUBLIC_SENTRY_DSN   the project's DSN from the Bugsink UI, set per build profile in app/eas.json. Empty or
//                            unset = reporting OFF: the app runs exactly as before, nothing is captured, nothing is sent.
//   EXPO_PUBLIC_ENV          production | preview (eas.json); a development build is always `dev`.
//
// Captured: native crashes (iOS/Android), unhandled JS errors and promise rejections, and the API client's failed
// requests as breadcrumbs (route pattern + method + status; never a body, never a query string). Everything passes
// crash.ts's scrubber before it leaves: no joke text, no transcripts, no search queries, no titles, no tokens.
// release = punchable@<version>+<build>, dist = <build>, so a stack trace matches the source map of that build.
import Constants from 'expo-constants';
import { Platform } from 'react-native';
import * as Sentry from '@sentry/react-native';
import { setApiEventListener } from './api';
import { apiBreadcrumb, environmentName, releaseName, scrubBreadcrumb, scrubEvent } from './crash';

export const DSN = (process.env.EXPO_PUBLIC_SENTRY_DSN || '').trim();
export const enabled = DSN.length > 0;

const version = Constants.expoConfig?.version ?? (Constants as any).nativeAppVersion ?? undefined;
const build = Platform.OS === 'web' ? undefined : (Constants as any).nativeBuildVersion ?? undefined;
export const release = releaseName(version, build);
export const environment = environmentName(process.env.EXPO_PUBLIC_ENV, typeof __DEV__ !== 'undefined' && __DEV__);

let started = false;

/** Called once at module load (the root layout imports this file first). Safe to call again; never throws. */
export function start(): boolean {
  if (started || !enabled) return started;
  try {
    Sentry.init({
      dsn: DSN,
      release,
      dist: build ? String(build) : undefined,
      environment,
      sendDefaultPii: false,
      maxBreadcrumbs: 30,
      // a crash is what we want to know about; performance tracing and replays would carry his screens out
      tracesSampleRate: 0,
      enableAutoPerformanceTracing: false,
      enableCaptureFailedRequests: false, // api.ts reports its own failures, as route patterns
      attachStacktrace: false,
      enableAutoSessionTracking: true,
      beforeSend: (event) => scrubEvent(event),
      beforeBreadcrumb: (bc) => scrubBreadcrumb(bc),
    });
    started = true;
  } catch {
    started = false; // a broken SDK must never take the app down with it
  }
  if (started) {
    setApiEventListener((ev) => {
      try {
        Sentry.addBreadcrumb(apiBreadcrumb(ev.method, ev.path, ev.status, ev.reason) as any);
      } catch {}
    });
  }
  return started;
}

/** Sentry's error boundary + native wrapper around the root component; the identity when reporting is off. */
export function wrapRoot<P extends object>(component: React.ComponentType<P>): React.ComponentType<P> {
  if (!started) return component;
  try {
    return Sentry.wrap(component as any) as React.ComponentType<P>;
  } catch {
    return component;
  }
}

/** A handled error worth knowing about (a screen's catch block). No-op when reporting is off. */
export function report(e: unknown, where?: string) {
  if (!started) return;
  try {
    Sentry.withScope((scope) => {
      if (where) scope.setTag('where', where.slice(0, 40));
      Sentry.captureException(e instanceof Error ? e : new Error(String(e).slice(0, 200)));
    });
  } catch {}
}

start();
