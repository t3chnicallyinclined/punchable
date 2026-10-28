// The app's icons on the phone: SF Symbols (expo-symbols SymbolView) under the same names the web draws as SVG, so
// "every icon matches the adjacent text weight" (HIG/sf-symbols: each symbol weight corresponds to a weight of SF).
// Filled variants where the HIG asks for them (tab bars, the mic), outlines in toolbars and lists. Android (no SF
// Symbols) and an unmapped name fall back to the SVG.
import React from 'react';
import { Platform } from 'react-native';
import { SymbolView } from 'expo-symbols';
import type { SFSymbol } from 'sf-symbols-typescript';
import { C } from './theme';
import { Icon as SvgIcon, type IconName, type IconProps } from './IconSvg';

export { P, type IconName, type IconProps } from './IconSvg';

const SF: Record<IconName, SFSymbol> = {
  today: 'house', write: 'pencil', notebook: 'book', back: 'chevron.left', desk: 'doc.text', shows: 'mappin.and.ellipse',
  boards: 'rectangle.grid.1x2', calendar: 'calendar', notes: 'note.text', mic: 'mic.fill', upload: 'square.and.arrow.up',
  photos: 'photo.on.rectangle', lock: 'lock', unlock: 'lock.open', search: 'magnifyingglass', grip: 'line.3.horizontal',
  up: 'arrow.up', down: 'arrow.down', trash: 'trash', list: 'list.bullet', map: 'map', star: 'star', more: 'ellipsis',
  play: 'play.fill', user: 'person.crop.circle', tonight: 'moon.stars.fill', bits: 'square.grid.2x2.fill',
  sets: 'list.bullet.rectangle.portrait.fill', plus: 'plus', chat: 'text.bubble', chevron: 'chevron.right',
};

export function Icon({ name, size = 22, color = C.text, sw = 2 }: IconProps) {
  const sf = SF[name as IconName];
  const svg = <SvgIcon name={name} size={size} color={color} sw={sw} />;
  if (Platform.OS !== 'ios' || !sf) return svg;
  // the SVG's stroke width stands in for the weight: 2 = semibold beside body text, 2.4+ = bold beside bold text
  return <SymbolView name={{ ios: sf }} size={size} tintColor={color} type="monochrome" weight={sw >= 2.4 ? 'bold' : 'semibold'} style={{ width: size, height: size }} fallback={svg} />;
}
