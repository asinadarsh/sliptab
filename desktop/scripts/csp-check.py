"""Does the desktop page work under the Content-Security-Policy in tauri.conf.json?

Serves the built page (npm run build) to headless Chrome with the policy as a header, fakes the Tauri
backend with sample notes, opens a note with an image, and prints every policy violation. Tauri adds
the hash of the page's one inline script itself when it builds, so the {HASHES} placeholder stands in.

  python3 scripts/csp-check.py

Expect: violations: [] and body text has Note 1: True. Run it after changing the page or the policy.
"""
import sys, re, json, hashlib, base64, mimetypes, pathlib
from playwright.sync_api import sync_playwright
ROOT = pathlib.Path(__file__).resolve().parent.parent / 'build'
import json
_c = json.load(open(pathlib.Path(__file__).resolve().parent.parent / 'src-tauri' / 'tauri.conf.json'))['app']['security']['csp']
# The real policy plus what the test needs: the page's inline-script hash, and 'self' for the fake backend's fetch.
CSP_TEMPLATE = _c.replace("script-src 'self'", "script-src 'self' {HASHES}").replace("connect-src ipc: http://ipc.localhost", "connect-src ipc: http://ipc.localhost 'self'")
html = (ROOT/'index.html').read_text()
hashes = ["'sha256-%s'" % base64.b64encode(hashlib.sha256(m.encode()).digest()).decode()
          for m in re.findall(r'<script>(.*?)</script>', html, re.S)]
csp = CSP_TEMPLATE.replace('{HASHES}', ' '.join(hashes))
PNG = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==')
now = 1764300000000
def note(i, **k):
    d = dict(id=f"00000000-0000-4000-8000-{i:012d}", title=f"Note {i}", body=f"Note {i}\n- [ ] item **bold** https://example.com/a", pinned=i<4,
             created_at=now-i*1000, updated_at=now-i*1000, color=["#ffb703","#8ecae6",None][i%3], pin_order=i, attachments=[])
    d.update(k); return d
img = dict(id="a1", kind="image", mime="image/png", bytes=len(PNG), sha256="a"*64, name="shot.png", width=1, height=1, created_at=now)
notes = [note(1, attachments=[img]), note(2), note(3), note(4), note(5)]
saves = [note(9, pinned=False, link=dict(url="https://example.com/p", site="example", title="Post", description="d", thumbnail="https://img.example.com/t.png", folder=None, done=False, fetched=True))]
settings = dict(hotkey="Ctrl+Shift+Space", side="right", barPosition=.5, panelWidth=None, hoverOpen=True, checkUpdates=True, showBar=True, theme="light", layout="list", barColor=None, onboarded=True, mcpAccess="off")
ans = {"notes_list": notes, "notes_search": notes, "saves_list": saves, "saves_search": saves, "saves_today": [], "save_folders": [[[], []][0], []],
       "pins_unseen": [], "get_settings": settings, "sync_status": dict(email=None, beta=False, error=None, received=0, billing=False),
       "platform": "windows", "get_hotkey": dict(accelerator="Ctrl+Shift+Space", bound="Ctrl+Shift+Space", portal=False, error=None),
       "get_autostart": False, "mcp_info": dict(command="x", args=[], path="x")}
init = """
window.__violations = [];
document.addEventListener('securitypolicyviolation', e => window.__violations.push(e.violatedDirective + ' ' + e.blockedURI + ' ' + (e.sample||'')));
let cb = 1; const cbs = {};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener(){} };
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' }, windows: [], webviews: [] },
  transformCallback(f){ const id = cb++; cbs[id]=f; return id; }, unregisterCallback(id){ delete cbs[id]; },
  convertFileSrc(p){ return p; },
  async invoke(cmd, args){
    window.__calls = (window.__calls||[]); window.__calls.push(cmd);
    if (cmd === 'media_get') { const r = await fetch('/__png'); return await r.arrayBuffer(); }
    if (cmd.startsWith('plugin:event|listen')) return cb++;
    if (cmd.startsWith('plugin:')) return null;
    const a = window.__ANS__; if (cmd in a) return JSON.parse(JSON.stringify(a[cmd]));
    return null;
  }
};
""".replace('window.__ANS__', json.dumps(ans).join(['(',')']))
with sync_playwright() as p:
    b = p.chromium.launch(channel='chrome'); ctx = b.new_context(viewport={'width': 1100, 'height': 760}); pg = ctx.new_page()
    msgs = []
    pg.on('console', lambda m: msgs.append(m.type + ': ' + m.text[:200]) if m.type in ('error','warning') else None)
    pg.on('pageerror', lambda e: msgs.append('pageerror: ' + str(e)[:200]))
    def serve(route):
        u = route.request.url.split('app.test')[1].split('?')[0]
        if u == '/__png': return route.fulfill(status=200, body=PNG, content_type='image/png')
        f = ROOT/u.lstrip('/')
        if not f.is_file(): f = ROOT/'index.html'
        h = {'content-type': mimetypes.guess_type(str(f))[0] or 'text/html'}
        if csp: h['content-security-policy'] = csp
        route.fulfill(status=200, body=f.read_bytes(), headers=h)
    ctx.route('http://app.test/**', serve)
    ctx.route('https://img.example.com/**', lambda r: r.fulfill(status=200, body=PNG, content_type='image/png'))
    pg.add_init_script(init)
    pg.goto('http://app.test/'); pg.wait_for_timeout(1800)
    # open the panel list and a note with the image
    first = pg.locator('text=Note 1').first
    if first.count(): first.click(); pg.wait_for_timeout(1200)
    pg.screenshot(path='/tmp/csp/%s.png' % ('with' if csp else 'without'))
    print('calls:', sorted(set(pg.evaluate('window.__calls||[]')))[:6], '...')
    print('violations:', pg.evaluate('window.__violations'))
    print('messages:', [m for m in msgs if 'CSP' in m or 'Content Security' in m or 'pageerror' in m][:6])
    print('img loaded:', pg.evaluate("[...document.images].map(i=>i.src.slice(0,30)+':'+i.naturalWidth)"))
    print('body text has Note 1:', 'Note 1' in pg.inner_text('body'))
    b.close()
