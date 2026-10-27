// iOS/Android: a reorderable column (long-press-drag the grip; gesture-handler + Reanimated). The web version is
// DragList.tsx. ↑ ↓ buttons stay as the accessible fallback. Items may differ in height.
import React, { useCallback, useRef } from 'react';
import { LayoutChangeEvent, Platform, View } from 'react-native';
import { Gesture, GestureDetector } from 'react-native-gesture-handler';
import Animated, { runOnJS, useAnimatedStyle, useSharedValue, withTiming, SharedValue } from 'react-native-reanimated';
import { C } from './theme';
import { haptics } from './haptics';
import { Icon } from './ui';

type Props<T> = {
  items: T[];
  keyOf: (t: T) => string;
  render: (t: T, i: number, handle: React.ReactNode) => React.ReactNode;
  onReorder: (from: number, to: number) => void;
  testID?: string;
};

function targetIndex(tops: number[], heights: number[], a: number, dy: number) {
  'worklet';
  const center = tops[a] + heights[a] / 2 + dy;
  let t = 0;
  for (let i = 0; i < tops.length; i++) {
    if (i === a) continue;
    if (center > tops[i] + heights[i] / 2) t++;
  }
  return t;
}

function Item({ index, count, active, dy, tops, heights, onLayout, children, handleGesture, onDrop }: {
  index: number; count: number; active: SharedValue<number>; dy: SharedValue<number>; tops: SharedValue<number[]>; heights: SharedValue<number[]>;
  onLayout: (i: number, e: LayoutChangeEvent) => void; children: (handle: React.ReactNode) => React.ReactNode; handleGesture: ReturnType<typeof Gesture.Pan>;
  onDrop: (from: number, to: number) => void;
}) {
  const startY = useRef(0);
  const style = useAnimatedStyle(() => {
    const a = active.value;
    if (a < 0) return { transform: [{ translateY: withTiming(0, { duration: 120 }) }], zIndex: 0, opacity: 1 };
    if (a === index) return { transform: [{ translateY: dy.value }], zIndex: 20, opacity: 0.92 };
    const t = targetIndex(tops.value, heights.value, a, dy.value);
    const h = heights.value[a] || 0;
    let shift = 0;
    if (a < index && index <= t) shift = -h;
    if (t <= index && index < a) shift = h;
    return { transform: [{ translateY: withTiming(shift, { duration: 120 }) }], zIndex: 0, opacity: 1 };
  });
  // VoiceOver: the grip offers Move up / Move down as accessibility actions (no drag needed)
  const grip = (extra?: any) => (
    <View testID={`drag-${index}`} accessibilityLabel="Drag to reorder" accessibilityRole="adjustable" accessibilityHint="Swipe up or down with one finger to move this one" {...extra}
      accessibilityActions={[
        ...(index > 0 ? [{ name: 'moveUp', label: 'Move up' }] : []),
        ...(index < count - 1 ? [{ name: 'moveDown', label: 'Move down' }] : []),
      ]}
      onAccessibilityAction={(e) => {
        if (e.nativeEvent.actionName === 'moveUp' && index > 0) onDrop(index, index - 1);
        else if (e.nativeEvent.actionName === 'moveDown' && index < count - 1) onDrop(index, index + 1);
      }}
      style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center', ...(Platform.OS === 'web' ? { cursor: 'grab', touchAction: 'none', userSelect: 'none' } as any : null) }}>
      <Icon name="grip" color={C.mut} sw={3} />
    </View>
  );
  const handle = <GestureDetector gesture={handleGesture}>{grip()}</GestureDetector>;
  return (
    <Animated.View style={style} onLayout={(e) => onLayout(index, e)}>
      {children(handle)}
    </Animated.View>
  );
}

export function DragList<T>({ items, keyOf, render, onReorder, testID }: Props<T>) {
  const active = useSharedValue(-1);
  const dy = useSharedValue(0);
  const tops = useSharedValue<number[]>([]);
  const heights = useSharedValue<number[]>([]);
  const lay = useRef<{ y: number; h: number }[]>([]);
  const onLayout = useCallback((i: number, e: LayoutChangeEvent) => {
    lay.current[i] = { y: e.nativeEvent.layout.y, h: e.nativeEvent.layout.height };
    const n = items.length;
    if (lay.current.slice(0, n).filter(Boolean).length === n) {
      tops.value = lay.current.slice(0, n).map((x) => x.y);
      heights.value = lay.current.slice(0, n).map((x) => x.h);
    }
  }, [items.length, tops, heights]);
  const finish = useCallback((a: number, t: number) => {
    if (a >= 0 && t !== a) {
      haptics.selection();
      onReorder(a, t);
    }
  }, [onReorder]);
  const lift = useCallback(() => haptics.light(), []);
  const gestureFor = (i: number) => Gesture.Pan()
    .activateAfterLongPress(220)
    .onStart(() => {
      active.value = i;
      dy.value = 0;
      runOnJS(lift)();
    })
    .onUpdate((e) => {
      dy.value = e.translationY;
    })
    .onEnd(() => {
      const t = targetIndex(tops.value, heights.value, i, dy.value);
      runOnJS(finish)(i, t);
    })
    .onFinalize(() => {
      active.value = -1;
      dy.value = 0;
    });
  return (
    <View testID={testID}>
      {items.map((it, i) => (
        <Item key={keyOf(it)} index={i} count={items.length} active={active} dy={dy} tops={tops} heights={heights} onLayout={onLayout} handleGesture={gestureFor(i)} onDrop={finish}>
          {(handle) => render(it, i, handle)}
        </Item>
      ))}
    </View>
  );
}

export function move<T>(xs: T[], from: number, to: number): T[] {
  const a = xs.slice();
  const [x] = a.splice(from, 1);
  a.splice(to, 0, x);
  return a;
}
