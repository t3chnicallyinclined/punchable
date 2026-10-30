// Archived set lists (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §1): `GET /api/boards?archived=1`, each
// row with Unarchive (swipe, not red: it is not loss; the menu is the no-gesture path; Undo = archive again) and
// Delete (destructive, last, confirmed: terminal). Pushed from Sets' toolbar menu; the system Back reads "Sets".
import React, { useState } from 'react';
import { Platform } from 'react-native';
import { router } from 'expo-router';
import { ApiError, patch } from '@/lib/api';
import { actionSheet } from '@/lib/actionSheet';
import { BoardListItem, creative, useCreative } from '@/lib/creative';
import { haptics } from '@/lib/haptics';
import { RowMenu } from '@/lib/RowMenu';
import { SwipeRow } from '@/lib/SwipeRow';
import { fmtShortDate } from '@/rules/format';
import { C } from '@/lib/theme';
import { Empty, ErrorCard, List, ListRow, MenuItem, MenuSheet, Skeleton, useUI } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const enc = encodeURIComponent;
const ios = Platform.OS === 'ios'; // the system context menu owns the long press there

export default function Archived() {
  const ui = useUI();
  const b = useCreative<{ boards: BoardListItem[] }>('boards:archived', creative.archivedBoards);
  const [menu, setMenu] = useState<{ title: string; items: MenuItem[] } | null>(null);
  const rows = (b.data?.boards || []).filter((x) => x.archived !== false);
  const fail = (what: string, e: any) => ui.snack(e instanceof ApiError ? `${what}: ${e.message}` : 'Can’t reach Punchable. Nothing changed.');
  const open = (x: BoardListItem) => router.push(`/list/${enc(x.id)}` as any);
  const unarchive = async (x: BoardListItem) => {
    try {
      await creative.patchBoard(x.id, { unarchive: true });
      haptics.success();
      b.reload();
      ui.snack(`Back in Set lists: ${x.title}`, async () => { await creative.patchBoard(x.id, { archive: true }); b.reload(); });
    } catch (e: any) { fail('Couldn’t bring it back', e); }
  };
  const del = (x: BoardListItem) => actionSheet({ title: `Delete ${x.title}?`, message: 'The bits stay; only this list goes.', options: [
    { label: 'Delete the set list', destructive: true, onPress: async () => {
      try { await patch(`/api/boards/${enc(x.id)}`, { delete: true }); ui.snack(`Deleted ${x.title}`); b.reload(); } catch (e: any) { ui.snack(`Couldn't delete: ${e.message}`); }
    } },
    { label: 'Cancel', cancel: true },
  ] });
  const items = (x: BoardListItem): MenuItem[] => [
    { label: 'Open', icon: 'doc.text', onPress: () => open(x), testID: `archived-open-${x.id}` },
    { label: 'Unarchive', icon: 'tray.and.arrow.up', onPress: () => unarchive(x), testID: `archived-unarchive-${x.id}` },
    { label: 'Delete', icon: 'trash', onPress: () => del(x), destructive: true, testID: `archived-delete-${x.id}` },
  ];
  return (
    <Screen title="Archived" testID="archived" back backLabel="Sets" onRefresh={b.reload}>
      {b.error && !b.data ? <ErrorCard error={b.error} onRetry={b.reload} /> : null}
      {!b.data && !b.error && <Skeleton lines={4} height={22} testID="archived-skeleton" />}
      {/* HIG/writing: nothing to do here, so no button */}
      {b.data && !rows.length && <Empty text="Nothing archived." />}
      <List>
        {rows.map((x) => (
          <SwipeRow key={x.id} testID={`archived-swipe-${x.id}`} trailing={{ label: 'Unarchive', color: C.tabOff, onPress: () => unarchive(x), testID: `archived-swipe-unarchive-${x.id}` }}
            onLongPress={ios ? undefined : () => { haptics.light(); setMenu({ title: x.title, items: items(x) }); }}>
            <RowMenu href={`/list/${enc(x.id)}`} items={items(x)}>
              <ListRow testID={`archived-${x.id}`} title={x.title} sub={`${x.n} jokes · archived ${fmtShortDate(x.updated)}`} onPress={() => open(x)} />
            </RowMenu>
          </SwipeRow>
        ))}
      </List>
      <MenuSheet visible={!!menu} title={menu?.title} items={menu?.items || []} onClose={() => setMenu(null)} testID="archived-menu" />
    </Screen>
  );
}
