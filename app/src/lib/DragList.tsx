// Web: a reorderable column with plain pointer events on the grip (mouse, pen, touch). No gesture-handler or
// Reanimated on the web, so they stay out of the web bundle; iOS/Android use DragList.native.tsx.
import React, { useCallback, useRef, useState } from 'react';
import { LayoutChangeEvent, View } from 'react-native';
import { C } from './theme';
import { Icon } from './ui';

type Props<T> = {
  items: T[];
  keyOf: (t: T) => string;
  render: (t: T, i: number, handle: React.ReactNode) => React.ReactNode;
  onReorder: (from: number, to: number) => void;
  testID?: string;
};

function targetIndex(tops: number[], heights: number[], a: number, dy: number) {
  const center = tops[a] + heights[a] / 2 + dy;
  let t = 0;
  for (let i = 0; i < tops.length; i++) if (i !== a && center > tops[i] + heights[i] / 2) t++;
  return t;
}

export function DragList<T>({ items, keyOf, render, onReorder, testID }: Props<T>) {
  const lay = useRef<{ y: number; h: number }[]>([]);
  const start = useRef(0);
  const [drag, setDrag] = useState<{ a: number; dy: number } | null>(null);
  const onLayout = useCallback((i: number, e: LayoutChangeEvent) => {
    lay.current[i] = { y: e.nativeEvent.layout.y, h: e.nativeEvent.layout.height };
  }, []);
  const tops = () => items.map((_, i) => lay.current[i]?.y ?? 0);
  const heights = () => items.map((_, i) => lay.current[i]?.h ?? 0);
  const t = drag ? targetIndex(tops(), heights(), drag.a, drag.dy) : -1;
  const y = (e: any) => e.clientY ?? e.nativeEvent?.clientY ?? 0;
  return (
    <View testID={testID}>
      {items.map((it, i) => {
        let shift = 0;
        if (drag) {
          const h = heights()[drag.a] || 0;
          if (i === drag.a) shift = drag.dy;
          else if (drag.a < i && i <= t) shift = -h;
          else if (t <= i && i < drag.a) shift = h;
        }
        const handle = (
          <View testID={`drag-${i}`} accessibilityLabel="Drag to reorder"
            onPointerDown={(e: any) => { e.preventDefault?.(); e.currentTarget?.setPointerCapture?.(e.pointerId); start.current = y(e); setDrag({ a: i, dy: 0 }); }}
            onPointerMove={(e: any) => { if (drag?.a === i) setDrag({ a: i, dy: y(e) - start.current }); }}
            onPointerUp={(e: any) => {
              if (drag?.a !== i) return;
              const to = targetIndex(tops(), heights(), i, y(e) - start.current);
              setDrag(null);
              if (to !== i) onReorder(i, to);
            }}
            onPointerCancel={() => setDrag(null)}
            style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center', cursor: 'grab', touchAction: 'none', userSelect: 'none' } as any}>
            <Icon name="grip" color={C.mut} sw={3} />
          </View>
        );
        return (
          <View key={keyOf(it)} onLayout={(e) => onLayout(i, e)}
            style={{ transform: [{ translateY: shift }], zIndex: drag?.a === i ? 20 : 0, opacity: drag?.a === i ? 0.92 : 1, ...(drag && drag.a !== i ? { transitionProperty: 'transform', transitionDuration: '120ms' } as any : null) }}>
            {render(it, i, handle)}
          </View>
        );
      })}
    </View>
  );
}

export function move<T>(xs: T[], from: number, to: number): T[] {
  const a = xs.slice();
  const [x] = a.splice(from, 1);
  a.splice(to, 0, x);
  return a;
}
