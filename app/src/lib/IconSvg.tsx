// The app's icons on the web: one SVG path per name (24-unit grid, stroked). On the phone the same names draw as SF
// Symbols (Icon.native.tsx), so a screen asks for `<Icon name="map" />` and gets the platform's glyph.
import React from 'react';
import Svg, { Path } from 'react-native-svg';
import { C } from './theme';

export const P = {
  today: 'M3 11l9-7 9 7v9h-6v-6H9v6H3z',
  write: 'M4 20h4L19 9l-4-4L4 16zM14 6l4 4',
  notebook: 'M6 3h11a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6zM6 3v18M10 8h5M10 12h5',
  back: 'M15 18l-6-6 6-6',
  desk: 'M5 4h10l4 4v12H5zM9 12h6M9 16h6',
  shows: 'M12 21s-7-6.2-7-12a7 7 0 0 1 14 0c0 5.8-7 12-7 12zM12 11.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z',
  boards: 'M4 5h16v4H4zM4 11h16v4H4zM4 17h10v3H4z',
  calendar: 'M3 5h18v16H3zM3 10h18M8 3v4M16 3v4',
  notes: 'M4 20h4L19 9l-4-4L4 16zM14 6l4 4',
  mic: 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM19 11a7 7 0 0 1-14 0M12 18v3',
  upload: 'M12 16V4M7 9l5-5 5 5M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3',
  photos: 'M4 6h12v12H4zM16 10l4-2v8l-4-2M8 10l3 2-3 2z',
  lock: 'M6 11h12v9H6zM8 11V8a4 4 0 0 1 8 0v3',
  unlock: 'M6 11h12v9H6zM8 11V8a4 4 0 0 1 7.5-2',
  search: 'M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM21 21l-5-5',
  grip: 'M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01',
  up: 'M12 19V5M6 11l6-6 6 6',
  down: 'M12 5v14M6 13l6 6 6-6',
  trash: 'M5 7h14M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3',
  list: 'M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01',
  map: 'M9 4L3 6v14l6-2 6 2 6-2V4l-6 2zM9 4v14M15 6v14',
  star: 'M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.5 2.9 1-6.1L3.2 9.5l6.1-.9z',
  more: 'M5 12h.01M12 12h.01M19 12h.01',
  play: 'M7 4l13 8-13 8z',
  record: 'M12 12m-8 0a8 8 0 1 0 16 0a8 8 0 1 0-16 0M12 12m-3 0a3 3 0 1 0 6 0a3 3 0 1 0-6 0',
  user: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0',
  // the three tabs (Mix* artboards): a spotlight, a grid of bits, a list
  tonight: 'M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
  bits: 'M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z',
  sets: 'M5 6h14M5 12h14M5 18h9',
  plus: 'M12 5v14M5 12h14',
  chat: 'M4 5h16v11H9l-5 4V5z',
  chevron: 'M9 6l6 6-6 6',
} as const;
export type IconName = keyof typeof P;

export type IconProps = { name: IconName | string; size?: number; color?: string; /** the stroke width; on the phone it picks the symbol weight */ sw?: number };

export function Icon({ name, size = 22, color = C.text, sw = 2 }: IconProps) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round">
      <Path d={(P as Record<string, string>)[name] || ''} />
    </Svg>
  );
}
