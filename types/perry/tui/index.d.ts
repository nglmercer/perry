// perry/tui — native TUI engine for Perry (#358).
//
// v0.2 surface (Phase 2): adds reactive state, keypress input, and the
// interactive `run()` render loop on top of Phase 1's Box / Text /
// render. Flexbox layout via Taffy is Phase 3; the wider widget set
// (Spacer, Input, TextArea, List, Select, Spinner, ProgressBar)
// lands in Phase 4.

declare module "perry/tui" {
    /**
     * Opaque widget handle returned by Box / Text. Pass to render(),
     * or to Box() as a child.
     */
    export type Widget = number & { readonly __perryTuiWidget: unique symbol };

    /**
     * Reactive state container. `.get()` returns the current value;
     * `.set(v)` writes a new value and triggers a re-render of the
     * `run()` loop on the next tick.
     */
    export interface State<T> {
        get(): T;
        set(value: T): void;
    }

    /**
     * Single-line text node.
     */
    export function Text(content: string): Widget;

    /**
     * Vertical stack of children. (Real flexbox layout — Taffy with
     * flexDirection / justifyContent / alignItems / flexGrow — lands
     * in Phase 3 of #358; for now Box is always a vertical stack.)
     */
    export function Box(): Widget;
    export function Box(children: Widget[]): Widget;

    /**
     * Paint one frame of `root` to stdout and return. Diffs against
     * the previous frame and emits only the cells that changed.
     * Use `run()` instead for interactive apps that re-render on
     * input or state change.
     */
    export function render(root: Widget): void;

    /**
     * Clear the screen and home the cursor. Called implicitly on
     * first render; exposed separately for callers that want explicit
     * setup before any render.
     */
    export function enter(): void;

    /**
     * Allocate a reactive state slot with the given initial value.
     */
    export function state<T>(initial: T): State<T>;

    /**
     * Register a keypress handler. The handler is called with the raw
     * byte sequence as a string — single ASCII bytes for printable
     * keys, multi-byte ANSI sequences for arrow keys / function keys
     * (e.g. `"\x1b[A"` for Up). Only one handler is supported in v1;
     * subsequent calls replace the prior handler.
     */
    export function useInput(handler: (input: string) => void): void;

    /**
     * Enter the interactive render loop. `component()` is called on
     * every state change; the returned widget tree is diffed and
     * painted with no flicker. Call `exit()` from a useInput handler
     * to leave the loop.
     */
    export function run(component: () => Widget): void;

    /**
     * Exit the render loop. The current frame finishes; raw mode is
     * restored and the alt screen is left before `run()` returns.
     */
    export function exit(): void;
}
