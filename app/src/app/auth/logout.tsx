// punchable://auth/logout (native) and /auth/logout (web): where Keycloak sends the browser after the RP-initiated
// sign-out (lib/auth.signOut, POST_LOGOUT_URI). Native: the auth session closes itself on this URL and signOut()
// continues to /login, so this screen draws nothing. Web: this device is already signed out; go to /login.
import { router } from 'expo-router';
import { useEffect } from 'react';
import { Platform } from 'react-native';
import '@/lib/auth';

export default function AuthLogout() {
  useEffect(() => {
    if (Platform.OS !== 'web') return;
    const t = setTimeout(() => router.replace('/login'), 50);
    return () => clearTimeout(t);
  }, []);
  return null;
}
