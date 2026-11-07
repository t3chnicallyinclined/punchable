// One piece of his material on a topic (SPEC 2026-11-06 §12.3 "A card"): his words on screen (a footnote head, up to
// three lines of the newest version, the punch in bold with its dot, "+ 2 more lines" unfolding in place), a trailing
// 44-pt Keep (+) or Kept (a soft check, never solid amber: the one solid amber on the page is the bottom button) or Put
// back, and under the words, once tapped: Open · To the rant · Drop. Tap the words again: Open. The same verbs are the
// long-press menu and VoiceOver's actions (HIG/context-menus, HIG/accessibility); swipes are extras.
import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { Icon } from './Icon';
import { haptics } from './haptics';
import { RowMenu, rowActions } from './RowMenu';
import { SwipeRow } from './SwipeRow';
import { C, F, linkColor } from './theme';
import { Btn } from './ui';
import { PROSE_CAP, ROW_CAP } from '@/rules/bitLines';
import { cardA11y, cardHead, cardLines, moreWord, type Mat, type State } from '@/rules/rantRules';

export type CardActs = {
  onKeep: () => void; onOpen: () => void; onToRant: (() => void) | null; onDrop: () => void; onPutBack: () => void;
};

export function MaterialCard({ m, state, selected, open, onSelect, onUnfold, acts, href, testID }: {
  m: Mat; state: State | null; selected: boolean; open: boolean; onSelect: (on: boolean) => void; onUnfold: () => void;
  acts: CardActs; href: string; testID?: string;
}) {
  const { lines, more, punch } = cardLines(m, open);
  const dropped = state === 'dropped';
  const kept = state === 'kept';
  const items = dropped
    ? [{ label: 'Open', icon: 'arrow.up.right', onPress: acts.onOpen }, { label: 'Put back', icon: 'arrow.uturn.backward', onPress: acts.onPutBack }]
    : [
      { label: 'Open', icon: 'arrow.up.right', onPress: acts.onOpen },
      kept ? { label: 'Un-keep', icon: 'minus.circle', onPress: acts.onKeep } : { label: 'Keep', icon: 'plus.circle', onPress: acts.onKeep },
      ...(acts.onToRant ? [{ label: 'To the rant', icon: 'text.badge.plus', onPress: acts.onToRant }] : []),
      { label: 'Drop', icon: 'xmark.circle', destructive: true, onPress: acts.onDrop },
    ];
  const tapWords = () => {
    if (selected) { acts.onOpen(); return; }
    haptics.selection();
    onSelect(true);
  };
  const body = (
    <View testID={testID} style={{ borderRadius: 14, backgroundColor: selected ? C.card2 : C.card, borderLeftWidth: 3, borderLeftColor: selected ? C.amber : 'transparent', paddingVertical: 10, paddingLeft: 12, paddingRight: 4 }}>
      <View style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 6 }}>
        <Pressable accessibilityRole="button" accessibilityLabel={cardA11y(m, state)} accessibilityState={{ selected }}
          accessibilityHint={selected ? 'Opens it' : 'Shows Open, To the rant and Drop'}
          {...rowActions(items)} onPress={tapWords} style={{ flex: 1, minWidth: 0, gap: 4, opacity: dropped ? 0.7 : 1 }}>
          <Text numberOfLines={1} maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 13, lineHeight: 18, color: C.mut2 }}>{cardHead(m)}</Text>
          {lines.map((l, i) => {
            const p = punch.includes(i);
            return (
              <View key={i} style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 6 }}>
                {p && <View style={{ width: 6, height: 6, borderRadius: 3, marginTop: 8, backgroundColor: C.amber }} />}
                <Text maxFontSizeMultiplier={PROSE_CAP} style={{ flex: 1, fontFamily: p ? F.bold : F.body, fontSize: 16, lineHeight: 22, color: C.text }}>{l}</Text>
              </View>
            );
          })}
        </Pressable>
        {dropped ? (
          <Pressable testID={testID ? `${testID}-putback` : undefined} accessibilityRole="button" accessibilityLabel="Put back" onPress={acts.onPutBack}
            style={({ pressed }) => ({ minHeight: 44, minWidth: 44, paddingHorizontal: 8, alignItems: 'center', justifyContent: 'center', opacity: pressed ? 0.6 : 1 })}>
            <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 15, color: linkColor() }}>Put back</Text>
          </Pressable>
        ) : (
          <Pressable testID={testID ? `${testID}-keep` : undefined} accessibilityRole="button" accessibilityLabel={kept ? 'Kept, tap to un-keep' : 'Keep'}
            accessibilityState={{ selected: kept }} onPress={() => { haptics.selection(); acts.onKeep(); }}
            style={({ pressed }) => ({ width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center', backgroundColor: kept ? C.amberSoft : 'transparent', opacity: pressed ? 0.6 : 1 })}>
            <Icon name={kept ? 'check' : 'plus'} size={20} color={kept ? linkColor() : C.mut2} sw={2.4} />
          </Pressable>
        )}
      </View>
      {more > 0 && (
        <Pressable accessibilityRole="button" accessibilityState={{ expanded: false }} onPress={onUnfold}
          style={({ pressed }) => ({ minHeight: 44, justifyContent: 'center', opacity: pressed ? 0.6 : 1 })}>
          <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 15, color: linkColor() }}>{moreWord(more)}</Text>
        </Pressable>
      )}
      {selected && !dropped && (
        <View accessibilityLabel="Actions for this card" style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6, paddingTop: 6 }}>
          <Btn small testID={testID ? `${testID}-open` : undefined} label="Open" onPress={acts.onOpen} />
          {acts.onToRant && <Btn small testID={testID ? `${testID}-torant` : undefined} label="To the rant" onPress={acts.onToRant} />}
          <Btn small testID={testID ? `${testID}-drop` : undefined} label="Drop" onPress={acts.onDrop} />
        </View>
      )}
    </View>
  );
  return (
    <SwipeRow testID={testID ? `${testID}-swipe` : undefined} a11yPassThrough
      leading={dropped ? null : kept ? { label: 'Un-keep', onPress: acts.onKeep } : { label: 'Keep', onPress: acts.onKeep }}
      trailing={dropped ? { label: 'Put back', onPress: acts.onPutBack } : { label: 'Drop', destructive: true, onPress: acts.onDrop }}>
      <RowMenu href={href} items={items}>{body}</RowMenu>
    </SwipeRow>
  );
}
