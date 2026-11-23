#!/usr/bin/env python3
"""Captures the real desktop panel with demo notes, for the website and store listings.

    python3 scripts/shots.py <out_dir>        (needs `npm run dev` running on :1420)

The page runs in Chromium with Tauri's IPC replaced by a fake holding the demo notes, so the
pixels are the app's own Svelte UI and CSS. Writes panel-light.png and panel-dark.png at 2x.
"""
import json
import sys
import time

from playwright.sync_api import sync_playwright

NOW = int(time.time() * 1000)
M = 60_000


def note(i, body, pinned=False, color=None, ago=5, order=None):
    return {"id": f"n{i}", "title": body.split("\n")[0].removeprefix("- [x] ").removeprefix("- [ ] "),
            "body": body, "pinned": pinned, "color": color, "created_at": NOW - (ago + 60) * M,
            "updated_at": NOW - ago * M, "pin_order": order, "attachments": []}


# The website's story: Jake needs the venue's number (the launch film), it gets pinned.
STORY = [
    note(1, "Venue for Saturday\nThe Courtyard · (415) 555-0142", True, "#ec4899", 1, 0),
    note(2, "Gate code\n4721#", True, "#f5a524", 40, 1),
    note(3, "Wi-Fi\nMaple-Guest · sunflower88", True, "#3b82f6", 90, 2),
    note(4, "Flight UA 482\nSFO to JFK · Fri 7:05 AM · Seat 14C", True, "#22c55e", 200, 3),
    note(5, "Groceries\n- [x] Oat milk\n- [ ] Avocados\n- [ ] Sourdough\n- [ ] Coffee beans", ago=12),
    note(6, "Dentist moved to Thursday 3:30 PM\nDr. Patel · (555) 014-2290", ago=48),
    note(7, "Gift ideas for Sam\nThat ceramic pour-over from the farmers market", ago=300),
]

NOTES = [
    note(1, "Gate code\n4721#", True, "#f5a524", 3, 0),
    note(2, "Wi-Fi\nMaple-Guest · sunflower88", True, "#3b82f6", 9, 1),
    note(3, "Flight UA 482\nSFO to JFK · Fri 7:05 AM · Seat 14C", True, "#22c55e", 30, 2),
    note(4, "Groceries\n- [x] Oat milk\n- [ ] Avocados\n- [ ] Sourdough\n- [ ] Coffee beans", ago=12),
    note(5, "Dentist moved to Thursday 3:30 PM\nDr. Patel · (555) 014-2290", ago=48),
    note(6, "Gift ideas for Sam\nThat ceramic pour-over from the farmers market", ago=300),
    note(7, "Q4 launch\nShip the pricing page before the Dec 3 review", ago=1800),
]

MOCK = """
(() => {
  const D = %s;
  let cb = 0;
  const reply = (cmd) => ({
    notes_list: D.notes, notes_search: D.notes, pins_unseen: [], notes_clean: 0,
    saves_list: [], saves_search: [], save_folders: [], saves_today: [],
    platform: { focusOnHover: true, fixedWindow: false, os: "linux" },
    get_settings: { hotkey: "Super+N", side: "right", barPosition: 0.35, panelWidth: null,
      hoverOpen: true, showBar: true, theme: D.theme, layout: "cards", barColor: null,
      onboarded: true, mcpAccess: "off" },
    get_hotkey: { accelerator: "Super+N", bound: null, portal: false, error: null },
    get_autostart: true,
    sync_status: { email: "emma.collins@gmail.com", lastSyncedAt: Date.now() - 60000, error: null,
      received: 0, beta: false, trialEndsAt: Date.now() + 26 * 864e5, paidUntil: null, billing: false },
    "plugin:app|version": "1.0.0",
  })[cmd] ?? null;
  window.__TAURI_INTERNALS__ = {
    invoke: async (cmd) => reply(cmd),
    transformCallback: () => ++cb,
    unregisterCallback: () => {},
    convertFileSrc: (s) => s,
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
})();
"""


def main() -> None:
    out = sys.argv[1]
    notes = STORY if sys.argv[2:] == ["story"] else NOTES
    with sync_playwright() as p:
        b = p.chromium.launch()
        for theme in ("light", "dark"):
            page = b.new_page(viewport={"width": 480, "height": 860}, device_scale_factor=2,
                              color_scheme=theme)
            page.add_init_script(MOCK % json.dumps({"notes": notes, "theme": theme}))
            page.on("console", lambda m: m.type == "error" and print("console:", m.text))
            page.goto("http://127.0.0.1:1420/")
            page.wait_for_timeout(1500)
            tag = f"{theme}-story" if notes is STORY else theme
            if notes is STORY:
                page.screenshot(path=f"{out}/strip-{tag}.png", omit_background=True)
                page.hover(".tab >> nth=0")  # the peek card of the first tab
                page.wait_for_timeout(600)
                page.screenshot(path=f"{out}/peek-{tag}.png", omit_background=True)
            page.hover(".bar")  # the panel slides open on hover
            page.wait_for_timeout(1200)
            page.mouse.move(240, 840)
            page.wait_for_timeout(400)
            page.screenshot(path=f"{out}/panel-{tag}.png", omit_background=True)
            page.close()
        b.close()


if __name__ == "__main__":
    main()
