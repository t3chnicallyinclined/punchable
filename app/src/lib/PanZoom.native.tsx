// iOS/Android: pan (one finger) and pinch (two) over the map, gesture-handler + Reanimated, reporting the view back
// to React on release (the transform animates on the UI thread meanwhile). The +/− buttons stay for VoiceOver.
import React, { useEffect } from 'react';
import { View } from 'react-native';
import { Gesture, GestureDetector } from 'react-native-gesture-handler';
import Animated, { runOnJS, useAnimatedStyle, useSharedValue } from 'react-native-reanimated';
import { C } from './theme';
import { ZoomButtons, clampScale } from './PanZoom';
import type { PanZoomProps, View_ } from './PanZoom';

export { clampScale, zoomAt, ZoomButtons } from './PanZoom';
export type { PanZoomProps, View_ } from './PanZoom';

export function PanZoom({ view, onView, width, height, children, minScale = 0.25, maxScale = 3, testID }: PanZoomProps) {
  const x = useSharedValue(view.x);
  const y = useSharedValue(view.y);
  const s = useSharedValue(view.scale);
  const sx = useSharedValue(0), sy = useSharedValue(0), ss = useSharedValue(1);
  useEffect(() => {
    x.value = view.x;
    y.value = view.y;
    s.value = view.scale;
  }, [view.x, view.y, view.scale]); // eslint-disable-line react-hooks/exhaustive-deps
  const report = (v: View_) => onView(v);
  const pan = Gesture.Pan().minPointers(1).maxPointers(1)
    .onStart(() => { sx.value = x.value; sy.value = y.value; })
    .onUpdate((e) => { x.value = sx.value + e.translationX; y.value = sy.value + e.translationY; })
    .onEnd(() => { runOnJS(report)({ x: x.value, y: y.value, scale: s.value }); });
  const pinch = Gesture.Pinch()
    .onStart((e) => { ss.value = s.value; sx.value = x.value; sy.value = y.value; })
    .onUpdate((e) => {
      const ns = clampScale(ss.value * e.scale, minScale, maxScale);
      const k = ns / ss.value;
      x.value = e.focalX - (e.focalX - sx.value) * k;
      y.value = e.focalY - (e.focalY - sy.value) * k;
      s.value = ns;
    })
    .onEnd(() => { runOnJS(report)({ x: x.value, y: y.value, scale: s.value }); });
  const style = useAnimatedStyle(() => ({ transform: [{ translateX: x.value }, { translateY: y.value }, { scale: s.value }] }));
  return (
    <View testID={testID} style={{ width, height, overflow: 'hidden', backgroundColor: C.bgDeep }}>
      <GestureDetector gesture={Gesture.Simultaneous(pan, pinch)}>
        <Animated.View style={[{ position: 'absolute', left: 0, top: 0, transformOrigin: 'top left' } as any, style]}>{children}</Animated.View>
      </GestureDetector>
      <ZoomButtons view={view} onView={onView} width={width} height={height} />
    </View>
  );
}
