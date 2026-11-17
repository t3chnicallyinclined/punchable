// '/' opens Bits (SPEC 2026-11-16 Slice 6: the Rant tab that answered '/' is gone; Bits is the first tab). Popped back
// to the stack's anchor, never a Redirect (that would leave two Bits screens and a Back on launch).
import { PopTo } from '@/lib/PopTo';

export default function Home() {
  return <PopTo href="/bits" />;
}
