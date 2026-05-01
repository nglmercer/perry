// perry/tui — native TUI engine for Perry (#358).
//
// v0.1 surface (Phase 1): Box + Text + render. Hooks (useState,
// useInput, useEffect) and the interactive render loop land in Phase 2;
// flexbox layout via Taffy lands in Phase 3; the wider widget set
// (Spacer, Input, TextArea, List, Select, Spinner, ProgressBar) lands
// in Phase 4. Each is purely additive — code written against this v0.1
// surface continues to work as later phases land.

declare module "perry/tui" {
    /**
     * Opaque widget handle returned by Box / Text. Pass to render(), or
     * to Box() as a child.
     */
    export type Widget = number & { readonly __perryTuiWidget: unique symbol };

    /**
     * Single-line text node.
     */
    export function Text(content: string): Widget;

    /**
     * Vertical stack of children. (Real flexbox layout — Taffy with
     * flexDirection / justifyContent / alignItems / flexGrow — lands in
     * Phase 3 of #358; for now Box is always a vertical stack.)
     */
    export function Box(): Widget;
    export function Box(children: Widget[]): Widget;

    /**
     * Paint one frame to stdout. Diffs against the previous frame and
     * emits only the cells that changed — never erases or rewrites the
     * full screen, so long views don't flicker.
     */
    export function render(root: Widget): void;

    /**
     * Clear the screen and home the cursor. Called implicitly on first
     * render; exposed separately for callers that want explicit setup
     * before any render (e.g. to print a banner first).
     */
    export function enter(): void;
}
