// An old URL that lands on a tab root (SPEC 2026-11-16 Slice 6: `/` -> Bits, `/filmed` -> Tapes). A <Redirect> would
// REPLACE the route on top of the tab stack's anchor and leave two copies of the root ([bits, bits], a Back on
// launch: iOS dev review); popping back to the root (dismissTo, a POP_TO that finds the anchor) leaves one.
import { useCallback } from 'react';
import { router, useFocusEffect } from 'expo-router';

export function PopTo({ href }: { href: string }) {
  useFocusEffect(useCallback(() => { router.dismissTo(href as any); }, [href]));
  return null;
}
