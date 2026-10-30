// https://punchable.io/auth/callback (web) and punchable://auth/callback (native): where Keycloak sends the code.
// Web: this page opens in the sign-in popup; importing @/lib/auth runs WebBrowser.maybeCompleteAuthSession(), which
// hands the code back to the login screen and closes the popup. If it was opened directly, go to /login.
// Native: a NO-OP. expo-router also routes the deep link here, but the code is delivered to the login screen's
// in-flight useAuthRequest by expo-web-browser's auth session; a redirect from here would unmount that request
// mid-exchange (the race). So this screen draws nothing and does nothing on the phone.
import { router } from 'expo-router';
import { useEffect } from 'react';
import { ActivityIndicator, Platform, View } from 'react-native';
import '@/lib/auth';
import { C } from '@/lib/theme';

export default function AuthCallback() {
  useEffect(() => {
    if (Platform.OS !== 'web') return;
    const t = setTimeout(() => router.replace('/login'), 1500);
    return () => clearTimeout(t);
  }, []);
  if (Platform.OS !== 'web') return null;
  return (
    <View style={{ flex: 1, backgroundColor: C.bg, alignItems: 'center', justifyContent: 'center' }}>
      <ActivityIndicator color={C.amber} accessibilityLabel="Finishing sign-in" />
    </View>
  );
}
