// The UI panel: the emulated client's screen beside the editor, drawn by
// the debug session (`wowlua/screenshot`), refreshed on every stop and
// while the game runs; clicks in it are clicks in the game (`wowlua/input`).
// It zooms (the wheel with Ctrl or Cmd, a pinch, its buttons) and zooms to
// a frame picked by name; zoomed in, the session draws it sharper.
const vscode = require("vscode");

class UiPanel {
    constructor(context) {
        this.context = context;
        this.panel = undefined;
        this.session = undefined;
        this.paused = false;
        this.ended = false;
        this.live = true;
        this.busy = false;
        this.timer = undefined;
        this.last = undefined; // { width, height, scale, at }
        this.highlight = undefined; // { x, y, width, height } screen pixels
        this.shots = 0;
        this.scale = 1; // the drawing's scale, as the panel's zoom asks
        this.frames = []; // the frames the picker offers: { label, rect }
        this.metrics = undefined; // the picture as displayed: { width, height, zoom }
    }

    /// A wowlua session started: the panel follows it.
    attach(session) {
        this.session = session;
        this.paused = false;
        this.ended = false;
        this.shots = 0;
        this.last = undefined;
        this.highlight = undefined;
        this.post({ type: "highlight", rect: null });
        if (vscode.workspace.getConfiguration("wowlua").get("ui.autoOpen")) {
            this.show(true);
        }
        this.post({ type: "status", text: "Running" });
        this.schedule();
    }

    detach(session) {
        if (this.session !== session) {
            return;
        }
        this.ended = true;
        this.stopTimer();
        this.post({ type: "status", text: "The session ended" });
    }

    stopped() {
        this.paused = true;
        this.stopTimer();
        this.post({ type: "status", text: "Paused (continue to click in the game)" });
        this.refresh();
    }

    continued() {
        this.paused = false;
        this.post({ type: "status", text: "Running" });
        this.schedule();
    }

    show(preserveFocus) {
        if (this.panel) {
            this.panel.reveal(vscode.ViewColumn.Beside, preserveFocus);
            return;
        }
        this.panel = vscode.window.createWebviewPanel(
            "wowlua.ui",
            "wowlua: UI",
            { viewColumn: vscode.ViewColumn.Beside, preserveFocus },
            { enableScripts: true, retainContextWhenHidden: true },
        );
        this.panel.webview.html = html();
        this.panel.webview.onDidReceiveMessage((m) => this.handleMessage(m));
        this.panel.onDidDispose(() => {
            this.panel = undefined;
            this.stopTimer();
        });
        this.post({ type: "frames", frames: this.frames });
        this.refresh();
    }

    post(message) {
        if (this.panel) {
            this.panel.webview.postMessage(message);
        }
    }

    interval() {
        return vscode.workspace.getConfiguration("wowlua").get("ui.refreshInterval");
    }

    schedule() {
        this.stopTimer();
        const ms = this.interval();
        if (!this.live || !ms || this.paused || this.ended || !this.session) {
            return;
        }
        this.timer = setInterval(() => this.refresh(), Math.max(200, ms));
    }

    stopTimer() {
        if (this.timer) {
            clearInterval(this.timer);
            this.timer = undefined;
        }
    }

    /// Asks the session for the screen, one request at a time.
    async refresh() {
        if (!this.panel || !this.session || this.ended || this.busy) {
            return;
        }
        this.busy = true;
        try {
            const scale = this.scale;
            const shot = await this.session.customRequest("wowlua/screenshot", scale === 1 ? {} : { scale });
            this.shots += 1;
            this.last = { width: shot.width, height: shot.height, scale, at: Date.now() };
            // The screen's size in its pixels, whatever the drawing's scale.
            this.post({
                type: "image",
                png: shot.png,
                width: shot.width / scale,
                height: shot.height / scale,
            });
            if (this.highlight) {
                this.post({ type: "highlight", rect: this.highlight });
            }
        } catch (e) {
            const text = String(e.message || e);
            // Before the session runs (its breakpoints still being set).
            this.post({ type: "status", text: text.includes("has not started") ? "Starting…" : `No picture: ${text}` });
        } finally {
            this.busy = false;
        }
    }

    /// Outlines a frame (screen pixels), as the Frames view selects it.
    outline(rect) {
        this.highlight = rect;
        this.show(true);
        this.post({ type: "highlight", rect });
    }

    /// Zooms to a frame: it fills the panel, outlined.
    zoomTo(rect) {
        this.highlight = rect;
        this.show(true);
        this.post({ type: "zoomTo", rect });
    }

    /// The frames the picker offers, from the Frames view's tree.
    setFrames(roots) {
        const out = [];
        const walk = (nodes, path) => {
            for (const n of nodes) {
                const name = n.name || "";
                if (n.visible && n.rect && n.rect.width > 0 && n.rect.height > 0 && name) {
                    out.push({ label: name, type: n.type, rect: n.rect });
                }
                walk(n.children || [], path);
            }
        };
        walk(roots, "");
        out.sort((a, b) => a.label.localeCompare(b.label));
        this.frames = out;
        this.post({ type: "frames", frames: out });
    }

    async handleMessage(m) {
        switch (m.type) {
            case "refresh":
                return this.refresh();
            case "live":
                this.live = !!m.on;
                return this.schedule();
            case "scale": {
                // The zoom wants a sharper (or lighter) drawing.
                const scale = Math.min(3, Math.max(1, Math.round(m.scale) || 1));
                if (scale !== this.scale) {
                    this.scale = scale;
                    return this.refresh();
                }
                return;
            }
            case "framesWanted":
                if (this.requestFrames) {
                    await this.requestFrames();
                }
                return;
            case "metrics":
                this.metrics = { width: m.width, height: m.height, zoom: m.zoom };
                return;
            case "pick": {
                const f = this.frames.find((x) => x.label === m.label);
                if (f) {
                    this.highlight = f.rect;
                    this.post({ type: "zoomTo", rect: f.rect });
                }
                return;
            }
            case "click": {
                if (!this.session || this.ended) {
                    return;
                }
                if (this.paused) {
                    this.post({ type: "status", text: "Paused: continue to click in the game" });
                    return;
                }
                try {
                    await this.session.customRequest("wowlua/input", {
                        action: "click",
                        x: Math.round(m.x),
                        y: Math.round(m.y),
                        button: m.button || "LeftButton",
                    });
                    this.post({ type: "status", text: `Clicked at ${Math.round(m.x)}, ${Math.round(m.y)}` });
                } catch (e) {
                    this.post({ type: "status", text: String(e.message || e) });
                }
                return this.refresh();
            }
        }
    }

    /// For tests: what the panel shows.
    state() {
        return {
            open: !!this.panel,
            shots: this.shots,
            last: this.last,
            paused: this.paused,
            ended: this.ended,
            scale: this.scale,
            frames: this.frames.length,
            metrics: this.metrics,
        };
    }
}

function nonce() {
    let s = "";
    const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    for (let i = 0; i < 32; i++) {
        s += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return s;
}

function html() {
    const n = nonce();
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'nonce-${n}';">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>wowlua: UI</title>
<style>
  html, body { height: 100%; }
  body { margin: 0; padding: 0; display: flex; flex-direction: column; background: var(--vscode-editor-background); color: var(--vscode-foreground); font-family: var(--vscode-font-family); font-size: var(--vscode-font-size); }
  .bar { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; padding: 6px 8px; background: var(--vscode-sideBar-background); border-bottom: 1px solid var(--vscode-panel-border); }
  button { background: var(--vscode-button-secondaryBackground); color: var(--vscode-button-secondaryForeground); border: none; padding: 3px 8px; cursor: pointer; min-width: 28px; }
  button:hover { background: var(--vscode-button-secondaryHoverBackground); }
  input[type=text] { background: var(--vscode-input-background); color: var(--vscode-input-foreground); border: 1px solid var(--vscode-input-border, transparent); padding: 3px 6px; width: 14em; }
  label { display: flex; gap: 4px; align-items: center; }
  .sep { width: 1px; align-self: stretch; background: var(--vscode-panel-border); }
  #zoomText { min-width: 3.5em; text-align: center; opacity: 0.85; }
  #status { margin-left: auto; opacity: 0.8; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 40%; }
  #stage { flex: 1; overflow: auto; position: relative; }
  #stage.panning, #stage.panning img { cursor: grabbing; }
  .wrap { position: relative; line-height: 0; margin: 8px; display: inline-block; }
  /* VS Code's webview styles cap images at the container (max-width:
     100%): zoomed in, that squeezed the screen sideways. */
  img { display: block; cursor: crosshair; max-width: none; max-height: none; }
  #outline { position: absolute; border: 2px solid var(--vscode-focusBorder); pointer-events: none; display: none; box-sizing: border-box; }
  #empty { padding: 24px; opacity: 0.7; line-height: 1.5; }
</style>
</head>
<body>
<div class="bar">
  <button id="refresh" title="Draw the screen again">Refresh</button>
  <label title="Draw the screen while the game runs"><input type="checkbox" id="live" checked> Live</label>
  <span class="sep"></span>
  <button id="out" title="Zoom out (Ctrl/Cmd + wheel)">−</button>
  <span id="zoomText">Fit</span>
  <button id="in" title="Zoom in (Ctrl/Cmd + wheel, or pinch)">+</button>
  <button id="fit" title="Fit the whole screen in the panel">Fit</button>
  <button id="one" title="The screen's own pixels">1:1</button>
  <span class="sep"></span>
  <input type="text" id="pick" list="frameList" placeholder="Zoom to frame…" title="Type a frame's name, then Enter">
  <datalist id="frameList"></datalist>
  <span id="status">Waiting for the session</span>
</div>
<div id="stage">
  <div id="empty">The game's screen shows here once the session runs.<br>Click in it to click in the game (right-click: the right button). Zoom with Ctrl/Cmd and the wheel or a pinch; pan with the scrollbars, the wheel, or by dragging with the middle button or Space held.</div>
  <div class="wrap" id="wrap" hidden><img id="screen" alt="The emulated client's screen" draggable="false"><div id="outline"></div></div>
</div>
<script nonce="${n}">
  const vscode = acquireVsCodeApi();
  const img = document.getElementById("screen");
  const wrap = document.getElementById("wrap");
  const stage = document.getElementById("stage");
  const status = document.getElementById("status");
  const outline = document.getElementById("outline");
  const zoomText = document.getElementById("zoomText");
  const pick = document.getElementById("pick");
  const list = document.getElementById("frameList");
  const MARGIN = 16;
  let screen = { width: 0, height: 0 }; // the game's screen, its pixels
  let zoom = null; // displayed pixels per screen pixel; null: fit
  let rect = null;
  let sentScale = 1;
  let pendingZoom = null; // a zoom asked for before the first picture

  function fitZoom() {
    if (!screen.width) return 1;
    return Math.max(0.05, Math.min((stage.clientWidth - MARGIN) / screen.width, (stage.clientHeight - MARGIN) / screen.height));
  }
  function current() { return zoom === null ? fitZoom() : zoom; }
  function layout() {
    if (!screen.width) return;
    const z = current();
    img.style.width = screen.width * z + "px";
    img.style.height = screen.height * z + "px";
    zoomText.textContent = zoom === null ? "Fit" : Math.round(z * 100) + "%";
    placeOutline();
    // (A timer: animation frames wait while the panel is hidden.)
    setTimeout(report, 0);
    // A sharper drawing once the screen shows larger than its pixels.
    const want = z * (window.devicePixelRatio || 1) > 2.5 ? 3 : z * (window.devicePixelRatio || 1) > 1.25 ? 2 : 1;
    if (want !== sentScale) { sentScale = want; vscode.postMessage({ type: "scale", scale: want }); }
  }
  // The displayed size, for the extension (its tests check the shape).
  function report() {
    const r = img.getBoundingClientRect();
    vscode.postMessage({ type: "metrics", width: r.width, height: r.height, zoom: current() });
  }
  function placeOutline() {
    if (!rect || !screen.width) { outline.style.display = "none"; return; }
    const z = current();
    outline.style.display = "block";
    outline.style.left = rect.x * z + "px";
    outline.style.top = rect.y * z + "px";
    outline.style.width = Math.max(2, rect.width * z) + "px";
    outline.style.height = Math.max(2, rect.height * z) + "px";
  }
  // Zooms to z keeping the screen point under (cx, cy) of the stage there.
  function zoomAt(z, cx, cy) {
    const before = current();
    const sx = (stage.scrollLeft + cx - MARGIN / 2) / before;
    const sy = (stage.scrollTop + cy - MARGIN / 2) / before;
    zoom = Math.min(16, Math.max(0.05, z));
    layout();
    stage.scrollLeft = sx * zoom + MARGIN / 2 - cx;
    stage.scrollTop = sy * zoom + MARGIN / 2 - cy;
  }
  function zoomBy(f) { zoomAt(current() * f, stage.clientWidth / 2, stage.clientHeight / 2); }
  // Zooms so a rectangle (screen pixels) fills the panel, centred.
  function zoomToRect(r) {
    if (!screen.width || !r) return;
    const z = Math.min(16, Math.max(0.05, 0.8 * Math.min(stage.clientWidth / Math.max(1, r.width), stage.clientHeight / Math.max(1, r.height))));
    zoom = z;
    layout();
    stage.scrollLeft = (r.x + r.width / 2) * z + MARGIN / 2 - stage.clientWidth / 2;
    stage.scrollTop = (r.y + r.height / 2) * z + MARGIN / 2 - stage.clientHeight / 2;
  }

  document.getElementById("refresh").onclick = () => vscode.postMessage({ type: "refresh" });
  document.getElementById("live").onchange = (e) => vscode.postMessage({ type: "live", on: e.target.checked });
  document.getElementById("in").onclick = () => zoomBy(1.25);
  document.getElementById("out").onclick = () => zoomBy(0.8);
  document.getElementById("fit").onclick = () => { zoom = null; layout(); stage.scrollLeft = 0; stage.scrollTop = 0; };
  document.getElementById("one").onclick = () => zoomAt(1, stage.clientWidth / 2, stage.clientHeight / 2);
  pick.addEventListener("change", () => { if (pick.value) vscode.postMessage({ type: "pick", label: pick.value }); });
  // Opening the picker asks for the frames shown now (windows open and
  // close while the game runs).
  pick.addEventListener("focus", () => vscode.postMessage({ type: "framesWanted" }));
  pick.addEventListener("mousedown", () => vscode.postMessage({ type: "framesWanted" }));
  pick.addEventListener("keydown", (e) => { if (e.key === "Enter" && pick.value) vscode.postMessage({ type: "pick", label: pick.value }); });

  // Ctrl or Cmd with the wheel, and a trackpad pinch (its wheel events
  // carry ctrlKey), zoom at the pointer; the wheel alone scrolls.
  stage.addEventListener("wheel", (e) => {
    if (!(e.ctrlKey || e.metaKey) || !screen.width) return;
    e.preventDefault();
    const r = stage.getBoundingClientRect();
    zoomAt(current() * Math.exp(-e.deltaY * 0.0025), e.clientX - r.left, e.clientY - r.top);
  }, { passive: false });

  // Panning: the middle button, or the left one with Space held.
  let space = false, pan = null;
  window.addEventListener("keydown", (e) => { if (e.code === "Space" && document.activeElement !== pick) { space = true; e.preventDefault(); } });
  window.addEventListener("keyup", (e) => { if (e.code === "Space") space = false; });
  stage.addEventListener("mousedown", (e) => {
    if (e.button === 1 || (e.button === 0 && space)) {
      e.preventDefault();
      pan = { x: e.clientX, y: e.clientY, left: stage.scrollLeft, top: stage.scrollTop, moved: false };
      stage.classList.add("panning");
    }
  });
  window.addEventListener("mousemove", (e) => {
    if (!pan) return;
    pan.moved = true;
    stage.scrollLeft = pan.left - (e.clientX - pan.x);
    stage.scrollTop = pan.top - (e.clientY - pan.y);
  });
  window.addEventListener("mouseup", () => { if (pan) { setTimeout(() => { pan = null; }, 0); stage.classList.remove("panning"); } });

  function point(e) {
    const r = img.getBoundingClientRect();
    return { x: (e.clientX - r.left) * screen.width / r.width, y: (e.clientY - r.top) * screen.height / r.height };
  }
  img.addEventListener("click", (e) => {
    if (pan || space) return;
    vscode.postMessage({ type: "click", button: "LeftButton", ...point(e) });
  });
  img.addEventListener("contextmenu", (e) => { e.preventDefault(); vscode.postMessage({ type: "click", button: "RightButton", ...point(e) }); });
  window.addEventListener("resize", () => { if (zoom === null) layout(); else placeOutline(); });

  window.addEventListener("message", (event) => {
    const m = event.data;
    if (m.type === "image") {
      const first = !screen.width;
      screen = { width: m.width, height: m.height };
      img.src = "data:image/png;base64," + m.png;
      document.getElementById("empty").hidden = true;
      wrap.hidden = false;
      if (first) layout(); else placeOutline();
      if (first && pendingZoom) { zoomToRect(pendingZoom); pendingZoom = null; }
    } else if (m.type === "status") {
      status.textContent = m.text;
    } else if (m.type === "highlight") {
      rect = m.rect;
      placeOutline();
    } else if (m.type === "zoomTo") {
      rect = m.rect;
      if (screen.width) zoomToRect(m.rect); else pendingZoom = m.rect;
    } else if (m.type === "frames") {
      list.innerHTML = "";
      for (const f of m.frames) {
        const o = document.createElement("option");
        o.value = f.label;
        o.label = f.type + " " + f.rect.width + "×" + f.rect.height;
        list.appendChild(o);
      }
    }
  });
</script>
</body>
</html>`;
}

module.exports = { UiPanel };
