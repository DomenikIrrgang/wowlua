// The UI panel: the emulated client's screen beside the editor, drawn by
// the debug session (`wowlua/screenshot`), refreshed on every stop and
// while the game runs; clicks in it are clicks in the game (`wowlua/input`).
// With Play on, the mouse and keyboard go to the game as when playing
// (decision T535): the mouse moved, its buttons held and let go, the wheel,
// keys held down by their binding names, text typed. It zooms (the wheel with Ctrl or Cmd, a pinch, its buttons) and zooms to
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
        this.playing = false; // Play: the mouse and keyboard go to the game
        this.inputs = []; // play input waiting to be sent, in order
        this.sending = false;
        this.sent = 0; // play input the session took
    }

    /// A wowlua session started: the panel follows it.
    attach(session) {
        this.session = session;
        this.paused = false;
        this.ended = false;
        this.shots = 0;
        this.last = undefined;
        this.highlight = undefined;
        this.inputs = [];
        this.post({ type: "highlight", rect: null });
        if (vscode.workspace.getConfiguration("wowlua").get("ui.autoOpen")) {
            this.show(true);
        }
        this.post({ type: "status", text: this.runningText() });
        this.schedule();
    }

    detach(session) {
        if (this.session !== session) {
            return;
        }
        this.ended = true;
        this.inputs = [];
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
        this.post({ type: "status", text: this.runningText() });
        this.schedule();
        // (keys and buttons let go during the stop)
        this.pump();
    }

    runningText() {
        return this.playing ? "Playing: the mouse and keyboard go to the game" : "Running";
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
        if (this.session && !this.ended) {
            this.post({ type: "status", text: this.paused ? "Paused (continue to click in the game)" : this.runningText() });
        }
        this.refresh();
        // Opened again during a session: drawing on as the game runs (its
        // timer stopped with the panel closed).
        this.schedule();
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
        // (playing: drawn as often as the session draws, PLAY_INTERVAL at most)
        const ms = this.playing ? Math.min(this.interval() || PLAY_INTERVAL, PLAY_INTERVAL) : this.interval();
        if (!this.live || !ms || this.paused || this.ended || !this.session) {
            return;
        }
        this.timer = setInterval(() => this.refresh(), Math.max(this.playing ? 50 : 200, ms));
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
            this.lastPng = shot.png;
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

    /// Play input, sent one request at a time in the order given; mouse
    /// moves waiting behind another collapse into the latest.
    give(input) {
        const last = this.inputs[this.inputs.length - 1];
        if (input.action === "mouseMove" && last && last.action === "mouseMove") {
            this.inputs[this.inputs.length - 1] = input;
        } else {
            this.inputs.push(input);
        }
        return this.pump();
    }

    async pump() {
        if (this.sending) {
            return;
        }
        this.sending = true;
        try {
            while (this.inputs.length && this.session && !this.ended && !this.paused) {
                const input = this.inputs.shift();
                try {
                    await this.session.customRequest("wowlua/input", input);
                    this.sent += 1;
                } catch (e) {
                    this.post({ type: "status", text: String(e.message || e) });
                }
                if (input.action !== "mouseMove") {
                    this.refresh();
                }
            }
        } finally {
            this.sending = false;
        }
    }

    async handleMessage(m) {
        switch (m.type) {
            case "refresh":
                return this.refresh();
            case "play":
                this.playing = !!m.on;
                if (this.session && !this.ended && !this.paused) {
                    this.post({ type: "status", text: this.runningText() });
                }
                return this.schedule();
            case "input": {
                const input = playInput(m.input);
                if (!input || !this.playing || !this.session || this.ended) {
                    return;
                }
                if (this.paused) {
                    // (a key or button let go reaches the game once it runs)
                    if (input.action === "keyUp" || input.action === "mouseUp") {
                        this.inputs.push(input);
                    } else if (input.action !== "mouseMove") {
                        this.post({ type: "status", text: "Paused: continue to play" });
                    }
                    return;
                }
                return this.give(input);
            }
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
            case "picker":
                this.picker = { open: m.open, items: m.items };
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
            picker: this.picker,
            playing: this.playing,
            sent: this.sent,
        };
    }
}

// How often the panel draws while playing, in milliseconds.
const PLAY_INTERVAL = 100;

/// The play input the panel may send (decision T535's `wowlua/input`
/// actions), checked and rounded; anything else is dropped.
function playInput(i) {
    if (!i || typeof i !== "object") {
        return undefined;
    }
    const at = () => Number.isFinite(i.x) && Number.isFinite(i.y);
    switch (i.action) {
        case "mouseMove":
            return at() ? { action: "mouseMove", x: Math.round(i.x), y: Math.round(i.y) } : undefined;
        case "mouseDown":
        case "mouseUp":
            return at() ? { action: i.action, x: Math.round(i.x), y: Math.round(i.y), right: !!i.right } : undefined;
        case "wheel":
            // (whole notches, as the game's OnMouseWheel gets them)
            return at() && Number.isFinite(i.delta) && i.delta !== 0
                ? { action: "wheel", x: Math.round(i.x), y: Math.round(i.y), delta: Math.sign(i.delta) * Math.min(10, Math.max(1, Math.round(Math.abs(i.delta)))) }
                : undefined;
        case "keyDown":
        case "keyUp":
            return typeof i.key === "string" && /^[A-Z0-9-]{1,40}$/.test(i.key) ? { action: i.action, key: i.key } : undefined;
        case "text":
            return typeof i.text === "string" && i.text.length > 0 && i.text.length <= 64 ? { action: "text", text: i.text } : undefined;
        default:
            return undefined;
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
  body.playing img { cursor: default; }
  #stage:focus { outline: none; }
  #outline { position: absolute; border: 2px solid var(--vscode-focusBorder); pointer-events: none; display: none; box-sizing: border-box; }
  #empty { padding: 24px; opacity: 0.7; line-height: 1.5; }
  .picker { position: relative; }
  /* The picker's list: the panel's own (a datalist closed whenever its
     options were renewed, as they are while it opens). */
  #pickList { position: absolute; z-index: 10; left: 0; top: 100%; min-width: 100%; max-height: 50vh; overflow-y: auto; margin: 2px 0 0; padding: 2px 0; list-style: none; background: var(--vscode-dropdown-background, var(--vscode-input-background)); color: var(--vscode-dropdown-foreground, var(--vscode-foreground)); border: 1px solid var(--vscode-dropdown-border, var(--vscode-panel-border)); box-shadow: 0 2px 8px var(--vscode-widget-shadow, rgba(0,0,0,0.4)); }
  #pickList[hidden] { display: none; }
  #pickList li { padding: 2px 8px; white-space: nowrap; cursor: pointer; display: flex; gap: 12px; justify-content: space-between; }
  #pickList li .kind { opacity: 0.6; }
  #pickList li.active { background: var(--vscode-list-activeSelectionBackground); color: var(--vscode-list-activeSelectionForeground); }
  #pickList li.none { opacity: 0.6; cursor: default; }
</style>
</head>
<body>
<div class="bar">
  <button id="refresh" title="Draw the screen again">Refresh</button>
  <label title="Draw the screen while the game runs"><input type="checkbox" id="live" checked> Live</label>
  <label title="The mouse and keyboard go to the game, as when playing: hover, click, drag, the wheel, keys held, typing"><input type="checkbox" id="play"> Play</label>
  <span class="sep"></span>
  <button id="out" title="Zoom out (Ctrl/Cmd + wheel)">−</button>
  <span id="zoomText">Fit</span>
  <button id="in" title="Zoom in (Ctrl/Cmd + wheel, or pinch)">+</button>
  <button id="fit" title="Fit the whole screen in the panel">Fit</button>
  <button id="one" title="The screen's own pixels">1:1</button>
  <span class="sep"></span>
  <span class="picker"><input type="text" id="pick" placeholder="Zoom to frame…" title="Pick a frame, or type part of its name" autocomplete="off" spellcheck="false"><ul id="pickList" hidden></ul></span>
  <span id="status">Waiting for the session</span>
</div>
<div id="stage" tabindex="0">
  <div id="empty">The game's screen shows here once the session runs.<br>Click in it to click in the game (right-click: the right button), or turn on Play to play: the mouse hovers, clicks and drags, the wheel scrolls, and keys run their key bindings (W walks) or type into the focused edit box. Zoom with Ctrl/Cmd and the wheel or a pinch; pan with the scrollbars, the wheel, or by dragging with the middle button or Space held.</div>
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
  const list = document.getElementById("pickList");
  let frames = []; // the picker's frames: { label, type, rect }
  let matches = [];
  let active = -1;
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
  // The picker: the frames whose names have what is typed, the list
  // drawn again (open) as the frames come.
  function renderList() {
    const q = pick.value.trim().toLowerCase();
    matches = frames.filter((f) => !q || f.label.toLowerCase().includes(q));
    if (active >= matches.length) active = matches.length - 1;
    list.textContent = "";
    if (!matches.length) {
      const li = document.createElement("li");
      li.className = "none";
      li.textContent = frames.length ? "No frame has that name" : "Loading the frames…";
      list.appendChild(li);
    }
    matches.slice(0, 300).forEach((f, i) => {
      const li = document.createElement("li");
      if (i === active) li.className = "active";
      const n = document.createElement("span");
      n.textContent = f.label;
      const k = document.createElement("span");
      k.className = "kind";
      k.textContent = f.type + " " + Math.round(f.rect.width) + "×" + Math.round(f.rect.height);
      li.append(n, k);
      // (mousedown: before the input's blur closes the list)
      li.addEventListener("mousedown", (e) => { e.preventDefault(); choose(f); });
      list.appendChild(li);
    });
  }
  function openList() { list.hidden = false; renderList(); }
  function closeList() { list.hidden = true; active = -1; }
  function choose(f) {
    pick.value = f.label;
    closeList();
    vscode.postMessage({ type: "pick", label: f.label });
  }
  // Opening the picker asks for the frames shown now (windows open and
  // close while the game runs).
  pick.addEventListener("focus", () => { openList(); vscode.postMessage({ type: "framesWanted" }); });
  pick.addEventListener("click", () => { if (list.hidden) openList(); });
  pick.addEventListener("input", () => { active = -1; openList(); });
  pick.addEventListener("blur", () => closeList());
  pick.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (list.hidden) openList();
      const n = Math.min(matches.length, 300);
      if (!n) return;
      active = e.key === "ArrowDown" ? (active + 1) % n : (active <= 0 ? n - 1 : active - 1);
      renderList();
      const el = list.children[active];
      if (el) el.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      const exact = frames.find((f) => f.label === pick.value);
      const f = active >= 0 ? matches[active] : exact || matches[0];
      if (f) choose(f);
    } else if (e.key === "Escape") {
      closeList();
    }
  });

  // Ctrl or Cmd with the wheel, and a trackpad pinch (its wheel events
  // carry ctrlKey), zoom at the pointer; the wheel alone scrolls.
  stage.addEventListener("wheel", (e) => {
    if (!(e.ctrlKey || e.metaKey) || !screen.width) return;
    e.preventDefault();
    const r = stage.getBoundingClientRect();
    zoomAt(current() * Math.exp(-e.deltaY * 0.0025), e.clientX - r.left, e.clientY - r.top);
  }, { passive: false });

  // Panning: the middle button, or the left one with Space held (not
  // while playing: Space is the game's).
  let space = false, pan = null;
  window.addEventListener("keydown", (e) => { if (e.code === "Space" && !playing && document.activeElement !== pick) { space = true; e.preventDefault(); } });
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
    if (pan || space || playing) return;
    vscode.postMessage({ type: "click", button: "LeftButton", ...point(e) });
  });
  img.addEventListener("contextmenu", (e) => { e.preventDefault(); if (!playing) vscode.postMessage({ type: "click", button: "RightButton", ...point(e) }); });

  // Play (decision T535): the mouse and keyboard as the game takes them.
  let playing = false;
  let held = null; // the mouse button held in the game: { right }
  let keys = new Map(); // keys held in the game: KeyboardEvent.code → name
  let moveAt = null; // a mouse move waiting for the next animation frame
  let notches = 0; // the wheel's turn not yet a whole notch
  const send = (input) => vscode.postMessage({ type: "input", input });
  function over(e) {
    const r = img.getBoundingClientRect();
    return !wrap.hidden && e.clientX >= r.left && e.clientX < r.right && e.clientY >= r.top && e.clientY < r.bottom;
  }
  function flushMove() { if (moveAt) { send({ action: "mouseMove", ...moveAt }); moveAt = null; } }
  // Lets go of what is held, as the game would see the keys come up.
  function release(at) {
    flushMove();
    for (const key of keys.values()) send({ action: "keyUp", key });
    keys.clear();
    if (held) { send({ action: "mouseUp", ...(at || { x: -1, y: -1 }), right: held.right }); held = null; }
  }
  function setPlaying(on) {
    if (!on) release();
    playing = on;
    document.body.classList.toggle("playing", on);
    vscode.postMessage({ type: "play", on });
    if (on) stage.focus();
  }
  document.getElementById("play").onchange = (e) => setPlaying(e.target.checked);
  img.addEventListener("mousedown", (e) => {
    if (!playing || (e.button !== 0 && e.button !== 2) || held) return;
    e.preventDefault();
    stage.focus();
    flushMove();
    held = { right: e.button === 2 };
    send({ action: "mouseDown", ...point(e), right: held.right });
  });
  window.addEventListener("mousemove", (e) => {
    if (!playing || pan || !(held || over(e))) return;
    const first = !moveAt;
    moveAt = point(e);
    if (first) requestAnimationFrame(flushMove);
  });
  window.addEventListener("mouseup", (e) => {
    if (!playing || !held || (e.button === 2) !== held.right) return;
    flushMove();
    send({ action: "mouseUp", ...point(e), right: held.right });
    held = null;
  });
  stage.addEventListener("wheel", (e) => {
    if (!playing || e.ctrlKey || e.metaKey || !over(e)) return;
    e.preventDefault();
    // (a notch: about 100 pixels, or 3 lines; up is +1 to the game)
    notches -= e.deltaMode === 1 ? e.deltaY / 3 : e.deltaY / 100;
    const whole = Math.trunc(notches);
    if (whole) { notches -= whole; flushMove(); send({ action: "wheel", ...point(e), delta: whole }); }
  }, { passive: false });
  // A key as WoW's bindings name it (SHIFT-1, ESCAPE), by its place on the
  // keyboard; modifiers in the client's order (ALT-, CTRL-, SHIFT-, META-).
  const NAMED = { Escape: "ESCAPE", Enter: "ENTER", NumpadEnter: "ENTER", Tab: "TAB", Space: "SPACE", Backspace: "BACKSPACE", Delete: "DELETE",
    ArrowUp: "UP", ArrowDown: "DOWN", ArrowLeft: "LEFT", ArrowRight: "RIGHT", Home: "HOME", End: "END", PageUp: "PAGEUP", PageDown: "PAGEDOWN" };
  function wowKey(e) {
    let name = NAMED[e.code];
    if (!name) {
      const m = /^(?:Key([A-Z])|Digit([0-9])|(F[0-9]{1,2}))$/.exec(e.code);
      if (!m) return null;
      name = m[1] || m[2] || m[3];
    }
    return (e.altKey ? "ALT-" : "") + (e.ctrlKey ? "CTRL-" : "") + (e.shiftKey ? "SHIFT-" : "") + (e.metaKey ? "META-" : "") + name;
  }
  window.addEventListener("keydown", (e) => {
    if (!playing || document.activeElement === pick) return;
    const name = wowKey(e);
    const text = e.key.length === 1 && !e.ctrlKey && !e.metaKey ? e.key : null;
    if (!name && !text) return;
    e.preventDefault();
    if (name && !e.repeat && !keys.has(e.code)) { keys.set(e.code, name); send({ action: "keyDown", key: name }); }
    if (text) send({ action: "text", text });
  });
  window.addEventListener("keyup", (e) => {
    if (!keys.has(e.code)) return;
    send({ action: "keyUp", key: keys.get(e.code) });
    keys.delete(e.code);
  });
  // (keys and the button held as the panel loses the keyboard come up)
  window.addEventListener("blur", () => { if (playing) release(); });
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
      frames = m.frames;
      if (!list.hidden) renderList();
    } else if (m.type === "openPicker") {
      // For the extension's tests: the picker opened as a click opens it,
      // what it shows reported once the frames came.
      openList();
      vscode.postMessage({ type: "framesWanted" });
      setTimeout(() => vscode.postMessage({ type: "picker", open: !list.hidden, items: list.querySelectorAll("li:not(.none)").length }), 1500);
    }
  });
</script>
</body>
</html>`;
}

module.exports = { UiPanel, playInput };
