// The input bar once rode the tab bar as its bottom accessory on iOS 26 (NativeTabs.BottomAccessory). There is no tab
// bar now (SPEC 2026-11-17 one notebook), so there is no accessory to draw it: the bar is always the screen's own dock
// (lib/Screen.native). With this true and nothing drawing the accessory, a conversation's input bar was not drawn at
// all on iOS 26 (SPEC 2026-11-16 §5 R11); tests/notebookShell.test.mjs pins it false.
export const ACCESSORY = false;
