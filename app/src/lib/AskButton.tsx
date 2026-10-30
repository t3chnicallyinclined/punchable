// The Ask button in the header of Write, Boards and Notebook (via Avatar in ui.tsx). Its own file so ui.tsx doesn't
// import ask.tsx (which imports ui.tsx).
import React from 'react';
import { Pressable, Text } from 'react-native';
import { router } from 'expo-router';
import Svg, { Path } from 'react-native-svg';
import { C, F } from './theme';

export function AskButton() {
  return (
    <Pressable testID="ask-open" accessibilityRole="button" accessibilityLabel="Ask your notebook" onPress={() => router.push('/ask' as any)}
      style={(st: any) => ({ height: 44, minWidth: 44, paddingHorizontal: 8, borderRadius: 22, flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 6, backgroundColor: st.hovered ? C.surface2 : 'transparent' })}>
      <Svg width={22} height={22} viewBox="0 0 24 24" fill="none" stroke={C.amber} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <Path d="M4 5h16v11H10l-5 4v-4H4z" />
      </Svg>
      <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.semi, fontSize: 15, color: C.amber }}>Ask</Text>
    </Pressable>
  );
}
