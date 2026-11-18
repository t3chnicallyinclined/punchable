// /filmed (the Rant tab's Filmed list) lands on Tapes (SPEC 2026-11-16 Slice 6): every take, set and memo is one list
// there. Popped back to the stack's anchor (lib/PopTo), never a Redirect.
import { PopTo } from '@/lib/PopTo';

export default function Filmed() {
  return <PopTo href="/tapes" />;
}
