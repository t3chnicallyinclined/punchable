// A bit opened from a topic (SPEC 2026-11-06 §12.7, C18): the bit page itself, pushed in the Rant stack so Back
// returns to the topic and the tab never switches. The page reads `id`; its own pushes (On tape, a set) go where they
// always go.
export { default } from '../../../../(bits)/joke/[id]';
