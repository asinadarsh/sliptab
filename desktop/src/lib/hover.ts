// Collapsed ⇄ expanded behaviour of the edge bar. Pure logic, no DOM, so it can be tested with `node --test`.

export type Mode = "collapsed" | "expanded";
export type Reason = "hover" | "click" | "hotkey" | "leave" | "escape";

export interface Timers {
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

export interface EdgeOptions {
  /** How long the pointer must rest on the bar before it expands, so a passing mouse does nothing. */
  hoverDelay: number;
  /** Grace period after the pointer leaves the panel; re-entering cancels the collapse. */
  leaveDelay: number;
  onChange: (mode: Mode, reason: Reason) => void;
  timers?: Timers;
}

const realTimers: Timers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

export function createEdge({ hoverDelay, leaveDelay, onChange, timers = realTimers }: EdgeOptions) {
  let mode: Mode = "collapsed";
  let pending: unknown = null;
  let paused = false;

  const cancel = () => {
    if (pending !== null) timers.clear(pending);
    pending = null;
  };
  const go = (next: Mode, reason: Reason) => {
    cancel();
    if (next === mode) return;
    mode = next;
    onChange(next, reason);
  };
  const after = (ms: number, next: Mode, reason: Reason) => {
    cancel();
    pending = timers.set(() => {
      pending = null;
      go(next, reason);
    }, ms);
  };

  return {
    get mode() {
      return mode;
    },
    pointerEnter() {
      if (mode === "collapsed" && !paused) after(hoverDelay, "expanded", "hover");
      else cancel(); // back inside the panel: keep it open
    },
    pointerLeave() {
      if (mode === "expanded") after(leaveDelay, "collapsed", "leave");
      else cancel(); // passed over the bar too quickly
    },
    click() {
      go("expanded", "click");
    },
    hotkey() {
      go(mode === "expanded" ? "collapsed" : "expanded", "hotkey");
    },
    escape() {
      go("collapsed", "escape");
    },
    /** One-click pause: hover stops expanding. Click and hotkey still work. */
    setPaused(p: boolean) {
      paused = p;
      if (p) cancel();
    },
  };
}
