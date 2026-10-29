// Web: pan with the pointer (mouse, pen, touch), zoom with the wheel or the +/− buttons. The phone build uses
// PanZoom.native.tsx (gesture-handler pan + pinch). Both report the same view {x, y, scale} to the map.
import React, { useRef } from 'react';
import { Pressable, Text, View } from 'react-native';
import { C, F } from './theme';

export type View_ = { x: number; y: number; scale: number };
export type PanZoomProps = { view: View_; onView: (v: View_) => void; width: number; height: number; children: React.ReactNode; minScale?: number; maxScale?: number; testID?: string };

export function clampScale(s: number, lo = 0.25, hi = 3) {
  'worklet'; // PanZoom.native calls this inside the pinch gesture's worklet: a plain function there is a crash
  // ("[Worklets] Tried to synchronously call a Remote Function"), the app's crash on build 16's map (Bugsink, 2026-10-29)
  return Math.max(lo, Math.min(hi, s));
}
/** Zoom around a point (px, py in screen space) so what is under the pointer stays put. */
export function zoomAt(v: View_, factor: number, px: number, py: number, lo?: number, hi?: number): View_ {
  const s = clampScale(v.scale * factor, lo, hi);
  const k = s / v.scale;
  return { scale: s, x: px - (px - v.x) * k, y: py - (py - v.y) * k };
}

export function ZoomButtons({ view, onView, width, height }: { view: View_; onView: (v: View_) => void; width: number; height: number }) {
  const b = (label: string, f: () => void, a11y: string) => (
    <Pressable accessibilityRole="button" accessibilityLabel={a11y} onPress={f} style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center', backgroundColor: C.card, borderRadius: 10 }}>
      <Text style={{ fontFamily: F.g7, fontSize: 20, color: C.text }}>{label}</Text>
    </Pressable>
  );
  return (
    <View style={{ position: 'absolute', right: 12, bottom: 12, gap: 8 }}>
      {b('+', () => onView(zoomAt(view, 1.3, width / 2, height / 2)), 'Zoom in')}
      {b('−', () => onView(zoomAt(view, 1 / 1.3, width / 2, height / 2)), 'Zoom out')}
    </View>
  );
}

export function PanZoom({ view, onView, width, height, children, minScale, maxScale, testID }: PanZoomProps) {
  const drag = useRef<{ x: number; y: number; vx: number; vy: number; id: number } | null>(null);
  const moved = useRef(false);
  return (
    <View testID={testID} style={{ width, height, overflow: 'hidden', backgroundColor: C.bgDeep }}
      {...({ onWheel: (e: any) => {
        e.preventDefault?.();
        const r = e.currentTarget?.getBoundingClientRect?.() || { left: 0, top: 0 };
        onView(zoomAt(view, e.deltaY < 0 ? 1.12 : 1 / 1.12, e.clientX - r.left, e.clientY - r.top, minScale, maxScale));
      } } as any)}
      onPointerDown={(e: any) => { drag.current = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y, id: e.pointerId }; moved.current = false; }}
      onPointerMove={(e: any) => {
        const d = drag.current;
        if (!d || e.pointerId !== d.id) return;
        const dx = e.clientX - d.x, dy = e.clientY - d.y;
        // capture the pointer only once this is a drag: capturing on pointerdown retargets pointerup to this view,
        // and no Pressable inside (a bit's label) ever gets its tap
        if (!moved.current && Math.abs(dx) + Math.abs(dy) > 3) { moved.current = true; e.currentTarget?.setPointerCapture?.(e.pointerId); }
        if (moved.current) onView({ ...view, x: d.vx + dx, y: d.vy + dy });
      }}
      onPointerUp={() => { drag.current = null; }}
      onPointerCancel={() => { drag.current = null; }}>
      <View style={{ position: 'absolute', left: 0, top: 0, transform: [{ translateX: view.x }, { translateY: view.y }, { scale: view.scale }], transformOrigin: 'top left' } as any}>
        {children}
      </View>
      <ZoomButtons view={view} onView={onView} width={width} height={height} />
    </View>
  );
}
