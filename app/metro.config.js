// Expo's default Metro config, through Sentry's wrapper: it stamps every bundle and its source map with a Debug ID, so
// a stack trace from a TestFlight build resolves against the map uploaded for that exact build (infra/OBSERVABILITY.md
// "Crash reports"). Nothing else is changed; without EXPO_PUBLIC_SENTRY_DSN the app never talks to Sentry at all.
const { getSentryExpoConfig } = require('@sentry/react-native/metro');

module.exports = getSentryExpoConfig(__dirname);
