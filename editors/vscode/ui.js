// The UI panel: the emulated client's screen beside the editor, drawn by
// the debug session (`wowlua/screenshot`), refreshed on every stop and
// while the game runs; clicks in it are clicks in the game (`wowlua/input`).
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
        this.last = undefined; // { width, height, at }
        this.highlight = undefined; // { x, y, width, height }
        this.shots = 0;
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
        this.panel.webview.html = html(this.panel.webview);
        this.panel.webview.onDidReceiveMessage((m) => this.handleMessage(m));
        this.panel.onDidDispose(() => {
            this.panel = undefined;
            this.stopTimer();
        });
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
            const shot = await this.session.customRequest("wowlua/screenshot", {});
            this.shots += 1;
            this.last = { width: shot.width, height: shot.height, at: Date.now() };
            this.post({ type: "image", png: shot.png, width: shot.width, height: shot.height });
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

    async handleMessage(m) {
        switch (m.type) {
            case "refresh":
                return this.refresh();
            case "live":
                this.live = !!m.on;
                return this.schedule();
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

function html(webview) {
    const n = nonce();
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'nonce-${n}';">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>wowlua: UI</title>
<style>
  body { margin: 0; padding: 0; background: var(--vscode-editor-background); color: var(--vscode-foreground); font-family: var(--vscode-font-family); font-size: var(--vscode-font-size); }
  .bar { position: sticky; top: 0; display: flex; gap: 8px; align-items: center; padding: 6px 8px; background: var(--vscode-sideBar-background); border-bottom: 1px solid var(--vscode-panel-border); z-index: 1; }
  button { background: var(--vscode-button-secondaryBackground); color: var(--vscode-button-secondaryForeground); border: none; padding: 3px 10px; cursor: pointer; }
  button:hover { background: var(--vscode-button-secondaryHoverBackground); }
  label { display: flex; gap: 4px; align-items: center; }
  #status { margin-left: auto; opacity: 0.8; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .stage { position: relative; padding: 8px; }
  .wrap { position: relative; display: inline-block; line-height: 0; }
  img { display: block; cursor: crosshair; image-rendering: auto; }
  .fit img { max-width: 100%; height: auto; }
  #outline { position: absolute; border: 2px solid var(--vscode-focusBorder); pointer-events: none; display: none; box-sizing: border-box; }
  #empty { padding: 24px; opacity: 0.7; }
</style>
</head>
<body>
<div class="bar">
  <button id="refresh" title="Draw the screen again">Refresh</button>
  <label title="Draw the screen while the game runs"><input type="checkbox" id="live" checked> Live</label>
  <button id="zoom" title="Fit the panel, or the screen's own pixels">100%</button>
  <span id="status">Waiting for the session</span>
</div>
<div class="stage fit" id="stage">
  <div id="empty">The game's screen shows here once the session runs. Click in it to click in the game; right-click for the right button.</div>
  <div class="wrap" id="wrap" hidden><img id="screen" alt="The emulated client's screen"><div id="outline"></div></div>
</div>
<script nonce="${n}">
  const vscode = acquireVsCodeApi();
  const img = document.getElementById("screen");
  const wrap = document.getElementById("wrap");
  const stage = document.getElementById("stage");
  const status = document.getElementById("status");
  const outline = document.getElementById("outline");
  let size = { width: 1, height: 1 };
  let rect = null;
  document.getElementById("refresh").onclick = () => vscode.postMessage({ type: "refresh" });
  document.getElementById("live").onchange = (e) => vscode.postMessage({ type: "live", on: e.target.checked });
  const zoom = document.getElementById("zoom");
  zoom.onclick = () => {
    const fit = stage.classList.toggle("fit");
    zoom.textContent = fit ? "100%" : "Fit";
    place();
  };
  function point(e) {
    const r = img.getBoundingClientRect();
    return { x: (e.clientX - r.left) * size.width / r.width, y: (e.clientY - r.top) * size.height / r.height };
  }
  img.addEventListener("click", (e) => vscode.postMessage({ type: "click", button: "LeftButton", ...point(e) }));
  img.addEventListener("contextmenu", (e) => { e.preventDefault(); vscode.postMessage({ type: "click", button: "RightButton", ...point(e) }); });
  function place() {
    if (!rect) { outline.style.display = "none"; return; }
    const k = img.clientWidth / size.width;
    outline.style.display = "block";
    outline.style.left = rect.x * k + "px";
    outline.style.top = rect.y * k + "px";
    outline.style.width = Math.max(2, rect.width * k) + "px";
    outline.style.height = Math.max(2, rect.height * k) + "px";
  }
  window.addEventListener("resize", place);
  img.addEventListener("load", place);
  window.addEventListener("message", (event) => {
    const m = event.data;
    if (m.type === "image") {
      size = { width: m.width, height: m.height };
      img.src = "data:image/png;base64," + m.png;
      document.getElementById("empty").hidden = true;
      wrap.hidden = false;
    } else if (m.type === "status") {
      status.textContent = m.text;
    } else if (m.type === "highlight") {
      rect = m.rect;
      place();
    }
  });
</script>
</body>
</html>`;
}

module.exports = { UiPanel };
