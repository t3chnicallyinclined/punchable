// /tapes was a tab root (SPEC 2026-11-16 Slice 6). One notebook (SPEC 2026-11-17 §3): it opens Notebook with that filter
// on. Popped back to the stack's anchor (lib/PopTo), never a Redirect (that would leave two copies of the root).
import React from 'react';
import { PopTo } from '@/lib/PopTo';
import { OLD_ROOTS } from '@/rules/notebookRules';

export default function Old() {
  return <PopTo href={OLD_ROOTS['/tapes']} />;
}
