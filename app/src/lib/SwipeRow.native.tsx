// iOS/Android: a row with LABELLED swipe actions (HIG): leading (swipe right, e.g. Accept) and trailing (swipe left,
// e.g. Remove in red; a full swipe runs it, because Undo exists), plus an optional second trailing action (Swap).
// The pan starts after 12 pt sideways so the list still scrolls, and never fights the iOS edge back-swipe (it
// does not activate within 24 pt of the left edge). Gesture-handler + Reanimated, like DragList.native.
import React, { useCallback, useRef } from 'react';
import { Pressable, Text, View } from 'react-native';
import { Gesture, GestureDetector } from 'react-native-gesture-handler';
import Animated, { runOnJS, useAnimatedStyle, useSharedValue, withTiming } from 'react-native-reanimated';
import { haptics } from './haptics';
import { C, F } from './theme';
import type { SwipeAction, SwipeRowProps } from './SwipeRowBase';

export type { SwipeAction, SwipeRowProps } from './SwipeRowBase';
const W = 96;         // one action's width
const FULL = 180;     // past this a full swipe runs the main action on release

function ActionBtn({ a, align, testID }: { a: SwipeAction; align: 'left' | 'right'; testID?: string }) {
  const bg = a.color || (a.destructive ? C.red : C.amber);
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={a.label} onPress={a.onPress}
      style={{ width: W, alignItems: 'center', justifyContent: 'center', backgroundColor: bg, alignSelf: 'stretch' }}>
      <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.g7, fontSize: 13, color: a.destructive ? '#fff' : C.bg, textAlign: 'center' }}>{a.label}</Text>
    </Pressable>
  );
}

export function SwipeRow({ children, leading, trailing, second, testID, onLongPress }: SwipeRowProps) {
  const x = useSharedValue(0);
  const fired = useRef(false);
  const maxRight = leading ? W : 0;
  const maxLeft = (trailing ? W : 0) + (second ? W : 0);
  const close = useCallback(() => {
    x.value = withTiming(0, { duration: 160 });
  }, [x]);
  const run = useCallback((a: SwipeAction | null | undefined) => {
    if (!a) return;
    haptics.medium();
    a.onPress();
    close();
  }, [close]);
  const tick = useCallback(() => haptics.light(), []);
  const pan = Gesture.Pan()
    .activeOffsetX([-12, 12])
    .failOffsetY([-10, 10])
    .onBegin(() => {
      fired.current = false;
    })
    .onUpdate((e) => {
      if (e.absoluteX - e.translationX < 24 && e.translationX > 0) return; // the iOS back-swipe edge
      const t = e.translationX;
      const lim = t > 0 ? maxRight + 40 : -(maxLeft + 40);
      const over = Math.abs(t) > Math.abs(lim);
      x.value = over ? lim + (t - lim) * 0.3 : t;
      if (!fired.current && Math.abs(t) > FULL) {
        fired.current = true;
        runOnJS(tick)();
      }
    })
    .onEnd((e) => {
      const t = e.translationX;
      if (t > FULL && leading) runOnJS(run)(leading);
      else if (t < -FULL && trailing) runOnJS(run)(trailing);
      else if (t > W / 2 && leading) x.value = withTiming(maxRight, { duration: 160 });
      else if (t < -W / 2 && (trailing || second)) x.value = withTiming(-maxLeft, { duration: 160 });
      else x.value = withTiming(0, { duration: 160 });
    });
  const style = useAnimatedStyle(() => ({ transform: [{ translateX: x.value }] }));
  const leftStyle = useAnimatedStyle(() => ({ width: Math.max(0, x.value) }));
  const rightStyle = useAnimatedStyle(() => ({ width: Math.max(0, -x.value) }));
  return (
    <View testID={testID} style={{ overflow: 'hidden' }}>
      {leading && (
        <Animated.View style={[{ position: 'absolute', left: 0, top: 0, bottom: 0, flexDirection: 'row', overflow: 'hidden' }, leftStyle]}>
          <ActionBtn a={{ ...leading, onPress: () => run(leading) }} align="left" testID={leading.testID} />
        </Animated.View>
      )}
      {(trailing || second) && (
        <Animated.View style={[{ position: 'absolute', right: 0, top: 0, bottom: 0, flexDirection: 'row', justifyContent: 'flex-end', overflow: 'hidden' }, rightStyle]}>
          {second && <ActionBtn a={{ ...second, onPress: () => run(second) }} align="right" testID={second.testID} />}
          {trailing && <ActionBtn a={{ ...trailing, onPress: () => run(trailing) }} align="right" testID={trailing.testID} />}
        </Animated.View>
      )}
      <GestureDetector gesture={pan}>
        <Animated.View style={[{ backgroundColor: C.bg }, style]}>
          <Pressable onLongPress={onLongPress} delayLongPress={350} onPress={() => { if (x.value !== 0) close(); }} accessibilityActions={[
            ...(leading ? [{ name: 'leading', label: leading.label }] : []),
            ...(trailing ? [{ name: 'trailing', label: trailing.label }] : []),
            ...(second ? [{ name: 'second', label: second.label }] : []),
          ]} onAccessibilityAction={(e) => {
            const n = e.nativeEvent.actionName;
            if (n === 'leading') run(leading);
            else if (n === 'trailing') run(trailing);
            else if (n === 'second') run(second);
          }}>
            {children}
          </Pressable>
        </Animated.View>
      </GestureDetector>
    </View>
  );
}
