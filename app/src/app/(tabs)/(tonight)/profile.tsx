// You (from the avatar): who is signed in, how, and Sign out. Settings, Connections and Learned are one tap down.
// Name, email and photo come from the ID token's claims (src/lib/session.ts); nothing here calls the API.
import { router } from 'expo-router';
import { useState } from 'react';
import { View } from 'react-native';
import Svg, { Path } from 'react-native-svg';
import { signOut, useSession } from '@/lib/auth';
import { C } from '@/lib/theme';
import { Btn, Card, Face, List, ListRow, SignInBtn, T } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

export default function ProfilePage() {
  const sess = useSession();
  const p = sess.profile;
  const [leavingNow, setLeavingNow] = useState(false);

  return (
    <Screen title="You" back testID="profile">
      {!sess.enabled ? (
        <Card testID="profile-local" style={{ gap: 6 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 16 }}>
            <Face size={64} />
            <View style={{ flex: 1, gap: 2 }}>
              <T v="h2">This computer</T>
              <T v="mut">Local copy. Sign-in is off here, so there is no account to sign out of.</T>
            </View>
          </View>
        </Card>
      ) : sess.signedIn && p ? (
        <Card testID="profile-card" style={{ gap: 14 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 16 }}>
            <Face size={64} />
            <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
              <T v="h2" numberOfLines={2} testID="profile-name">{p.name || p.email || 'Signed in'}</T>
              {!!p.email && <T v="mut" numberOfLines={1} testID="profile-email">{p.email}</T>}
            </View>
          </View>
          {p.via && (
            <View testID="profile-via" style={{ flexDirection: 'row', alignItems: 'center', gap: 8, borderTopWidth: 1, borderTopColor: C.border, paddingTop: 12 }}>
              {p.via === 'google' && <GoogleG />}
              <T v="mut">{p.via === 'google' ? 'Signed in with Google' : 'Signed in with email'}</T>
            </View>
          )}
        </Card>
      ) : (
        <Card testID="profile-signed-out" style={{ gap: 12 }}>
          <T v="h2">You’re signed out</T>
          <SignInBtn testID="profile-signin" full />
        </Card>
      )}

      <Card style={{ gap: 0, paddingVertical: 4 }}>
        <List>
          <ListRow testID="open-settings" title="Settings & privacy" sub="Connections, AI, where your material goes, export" onPress={() => router.push('/settings' as any)} />
          <ListRow testID="open-learned" title="What I’ve learned about you" sub="The AI’s read of your taste, and how sure it is" onPress={() => router.push('/learned' as any)} />
        </List>
      </Card>

      {sess.enabled && sess.signedIn && (
        <View style={{ gap: 8 }}>
          <Btn testID="sign-out" label={leavingNow ? 'Signing out…' : 'Sign out'} disabled={leavingNow}
            onPress={() => {
              setLeavingNow(true);
              signOut().finally(() => setLeavingNow(false));
            }} />
          <T v="small" style={{ textAlign: 'center' }}>Signs you out on this device and on auth.punchable.io. Your material stays where it is.</T>
        </View>
      )}
    </Screen>
  );
}

function GoogleG() {
  return (
    <Svg width={18} height={18} viewBox="0 0 24 24" aria-hidden>
      <Path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z" />
      <Path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" />
      <Path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l3.66-2.84z" />
      <Path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" />
    </Svg>
  );
}
