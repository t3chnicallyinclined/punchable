// https://punchable.io/auth/callback (web) and punchable://auth/callback (native): where Keycloak sends the code.
// On the web this page opens in the sign-in popup; importing @/lib/auth runs WebBrowser.maybeCompleteAuthSession(),
// which hands the code back to the login screen and closes the popup. If it was opened directly, go to /login.
import { router } from 'expo-router';
import { useEffect } from 'react';
import { ActivityIndicator, View } from 'react-native';
import '@/lib/auth';
import { C } from '@/lib/theme';

export default function AuthCallback() {
  useEffect(() => {
    const t = setTimeout(() => router.replace('/login'), 1500);
    return () => clearTimeout(t);
  }, []);
  return (
    <View style={{ flex: 1, backgroundColor: C.bg, alignItems: 'center', justifyContent: 'center' }}>
      <ActivityIndicator color={C.amber} />
    </View>
  );
}
