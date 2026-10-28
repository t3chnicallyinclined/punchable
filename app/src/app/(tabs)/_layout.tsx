// The three tabs (docs/design/HIG.md §2.1). The group names (tabs), (tonight), (bits), (sets) never appear in a URL,
// so every deep link keeps working. Phone: the system tab bar (lib/TabsShell.native.tsx). Web: a Slot inside the
// existing custom chrome (lib/TabsShell.tsx).
export { TabsShell as default } from '@/lib/TabsShell';
