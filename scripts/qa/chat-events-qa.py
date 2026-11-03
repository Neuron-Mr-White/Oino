#!/usr/bin/env python3
"""[chat-events] QA shots on the real installed app (rig on the VM): every chat event
kind collapsed and expanded, desktop 1440x900 and phone 390x844.
   ce-qa.py <deb> <outdir>"""
import json, os, subprocess, sys, time
sys.path.insert(0, "/root/oino/scripts/perf")
import lib  # noqa: E402
from PIL import Image  # noqa: E402

deb, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
REPO = "/root/oino"
qa = "/root/qa-ce"
subprocess.run(["rm", "-rf", qa])
lib.seed_agent_dir(qa, 11434)
work = f"{qa}/work"
seed = subprocess.run(["node", f"{REPO}/scripts/qa/chat-events-seed.mjs", f"{qa}/agent", work], capture_output=True, text=True)
SFILE = seed.stdout.strip()
SID = SFILE.rsplit("_", 1)[1].removesuffix(".jsonl")
env = dict(os.environ, QA_HOME=qa, QA_DISPLAY=":151", QA_PORT="4451", QA_SIZE="1440x900", QA_MAX_RIGS="3",
           PATH="/root/.cargo/bin:/root/node/bin:/root/.local/bin:" + os.environ["PATH"])
R = {"sid": SID, "kinds": {}}


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def rig(*a):
    return subprocess.run(["bash", f"{REPO}/scripts/qa/rig.sh", *a], env=env, capture_output=True, text=True, timeout=300)


def mjs(*a):
    p = subprocess.run(["node", f"{REPO}/scripts/qa/rig.mjs", *a], env=env, capture_output=True, text=True, timeout=120)
    return p.stdout.strip() or p.stderr.strip()


def ev(js):
    o = mjs("eval", js)
    try:
        return json.loads(o)
    except Exception:
        return o


def shot(name, crop=None):
    p = mjs("shot", name)
    dst = f"{out}/{name}.png"
    try:
        im = Image.open(p)
        if crop:
            x, y, w, h = crop
            W, H = im.size
            im = im.crop((max(0, x - 12), max(0, y - 12), min(W, x + w + 12), min(H, y + h + 12)))
        im.save(dst, optimize=True)
    except Exception as e:  # noqa: BLE001
        log("shot fail", name, e, p)
    return dst


SCROLLER = "(document.querySelector('.conv-tl .tl') || document.querySelector('.tl'))"
ROWS = """const out=[];for(const r of document.querySelectorAll('[data-testid=tl-event]')){const h=r.querySelector('[data-testid=tl-event-head]');const b=r.getBoundingClientRect();
out.push({kind:r.dataset.event,title:h.title,open:h.getAttribute('aria-expanded'),y:Math.round(b.top),h:Math.round(b.height)});}return out;"""


def find_row(title):
    """Scrolls until the row with this head title is mounted; returns its rect."""
    js = f"""const t={json.dumps(title)};const r=[...document.querySelectorAll('[data-testid=tl-event-head]')].find(h=>h.title===t);
if(!r) return null; r.closest('[data-testid=tl-event]').scrollIntoView({{block:'center'}}); return true;"""
    for step in range(40):
        if ev(js) is True:
            time.sleep(0.5)
            return rect(title)
        ev(f"const s={SCROLLER}; s.scrollTop = {step} * 400; return s.scrollTop")
        time.sleep(0.3)
    return None


def rect(title):
    return ev(f"""const t={json.dumps(title)};const h=[...document.querySelectorAll('[data-testid=tl-event-head]')].find(h=>h.title===t);
if(!h) return null;const r=h.closest('[data-testid=tl-event]').getBoundingClientRect();return [Math.round(r.left),Math.round(r.top),Math.round(r.width),Math.round(r.height)]""")


def click_head(title):
    return ev(f"""const t={json.dumps(title)};const h=[...document.querySelectorAll('[data-testid=tl-event-head]')].find(h=>h.title===t);
if(!h) return null;h.click();return h.getAttribute('aria-expanded')""")


fake = subprocess.Popen([sys.executable, f"{REPO}/scripts/qa/phone-fakellm.py"], env=dict(os.environ, FAKELLM_LOG=f"{out}/fakellm.log"))
try:
    log("seed", SFILE)
    log("rig up", rig("up", deb).stdout[-300:])
    sock = f"{qa}/agent/host/app.sock"
    lib.wait_sock(sock, 120)
    for _ in range(120):
        if ev("return document.querySelectorAll('[data-testid], nav, aside').length > 5") is True:
            break
        time.sleep(0.5)
    time.sleep(4)
    href = ev("const a=document.querySelector('a[href*=\"#/s/\"]');return a?a.getAttribute('href'):null")
    log("href", href)
    hid = href.split("#/s/")[1].split("/")[0] if isinstance(href, str) and "#/s/" in href else "local"
    ev(f"location.hash = '#/s/{hid}/{SID}'; return location.hash")
    for _ in range(60):
        if ev("return document.querySelectorAll('[data-testid=tl-event]').length") not in (0, None, "0"):
            break
        time.sleep(0.5)
    time.sleep(3)
    shot("d-00-chat-1440")
    # collect every row title over the whole chat
    titles = []
    ev(f"const s={SCROLLER}; s.scrollTop=0; return 0")
    time.sleep(1)
    for step in range(60):
        for r in ev(ROWS) or []:
            if isinstance(r, dict) and r["title"] not in [t for t, _ in titles]:
                titles.append((r["title"], r["kind"]))
        done = ev(f"const s={SCROLLER}; const before=s.scrollTop; s.scrollTop += 500; return s.scrollTop===before")
        time.sleep(0.4)
        if done is True:
            break
    log("rows", len(titles), [k for _, k in titles])
    R["rows"] = [{"title": t, "kind": k} for t, k in titles]
    # desktop: each turn collapsed (scroll by turns), then each row expanded (cropped)
    nturns = ev("return document.querySelectorAll('.tl-turn').length")
    ev(f"const s={SCROLLER}; s.scrollTop=0; return 0")
    time.sleep(1)
    for i in range(12):
        shot(f"d-collapsed-{i:02d}-1440")
        done = ev(f"const s={SCROLLER}; const b=s.scrollTop; s.scrollTop += s.clientHeight - 120; return s.scrollTop===b")
        time.sleep(0.8)
        if done is True:
            break
    seen = {}
    for title, kind in titles:
        seen[kind] = seen.get(kind, 0) + 1
        tag = f"{kind}" + (f"-{seen[kind]}" if seen[kind] > 1 else "")
        rc = find_row(title)
        if not rc:
            log("missing", kind, title)
            continue
        R["kinds"].setdefault(tag, {})["collapsed_h"] = rc[3]
        shot(f"d-{tag}-collapsed", crop=rc)
        st = click_head(title)
        time.sleep(0.9)
        ev(f"""const t={json.dumps(title)};const h=[...document.querySelectorAll('[data-testid=tl-event-head]')].find(h=>h.title===t);
if(h) h.closest('[data-testid=tl-event]').scrollIntoView({{block:'start'}}); {SCROLLER}.scrollTop -= 60; return 1""")
        time.sleep(0.6)
        rc2 = rect(title)
        body = ev(f"""const t={json.dumps(title)};const h=[...document.querySelectorAll('[data-testid=tl-event-head]')].find(h=>h.title===t);
const r=h&&h.closest('[data-testid=tl-event]');const b=r&&r.querySelector('.tl-event-body');return b?{{fmt:b.dataset.format||(b.classList.contains('tl-event-render')?'render':''),len:b.textContent.length,sh:b.scrollHeight,ch:b.clientHeight}}:null""")
        R["kinds"][tag].update({"expanded": st, "expanded_h": rc2[3] if rc2 else None, "body": body})
        if rc2:
            x, y, w, h = rc2
            h = min(h, 900 - max(0, y))
            shot(f"d-{tag}-expanded", crop=(x, y, w, h))
        click_head(title)
        time.sleep(0.4)
    R["desktop_overflow"] = ev("return document.documentElement.scrollWidth > innerWidth")
    # phone 390
    mjs("size", "390", "844")
    time.sleep(3)
    R["phone_shell"] = ev("return document.documentElement.dataset.shell")
    ev(f"const s={SCROLLER}; s.scrollTop=0; return 0")
    time.sleep(1)
    for i in range(30):
        shot(f"p-collapsed-{i:02d}-390")
        done = ev(f"const s={SCROLLER}; const b=s.scrollTop; s.scrollTop += s.clientHeight - 100; return s.scrollTop===b")
        time.sleep(0.8)
        if done is True:
            break
    R["phone_overflow"] = ev("return document.documentElement.scrollWidth > innerWidth")
    R["phone_rows"] = ev("""return [...document.querySelectorAll('[data-testid=tl-event-head]')].map(h=>Math.round(h.getBoundingClientRect().height))""")
    seen = {}
    for title, kind in titles:
        seen[kind] = seen.get(kind, 0) + 1
        tag = f"{kind}" + (f"-{seen[kind]}" if seen[kind] > 1 else "")
        rc = find_row(title)
        if not rc:
            log("phone missing", kind)
            continue
        st = click_head(title)
        if st is None:
            continue
        time.sleep(1.2)
        sheet = ev("return !!document.querySelector('[data-testid=event-sheet]')")
        R["kinds"].setdefault(tag, {})["phone_sheet"] = sheet
        shot(f"p-{tag}-sheet")
        mjs("key", "Escape")
        time.sleep(0.8)
        if ev("return !!document.querySelector('[data-testid=event-sheet]')") is True:
            ev("history.back(); return 1")
            time.sleep(0.8)
finally:
    with open(f"{out}/result.json", "w") as f:
        json.dump(R, f, indent=1)
    rig("down")
    fake.terminate()
    log("done")
