// Web/Android resolution of './Icon'. The phone resolves './Icon' to Icon.native.tsx, which must import IconSvg
// directly: importing './Icon' from Icon.native.tsx is a self-import (Metro picks the .native file) and recursed until
// the stack overflowed, which took every screen down on the phone (2026-10-28, TestFlight builds 4-6).
export * from './IconSvg';
