// Drives VS Code's debugger with the wowlua extension as a user would
// (decisions T358, T359): F5 on the widget demo's counter test stops at the
// OnClick breakpoint with its line shown; a stop in Blizzard's UI as it
// loads shows that file; the UI panel draws the running addon, a click in it
// stops at the breakpoint, and the Frames view outlines a frame. Needs a WoW
// installation (camelot's Blizzard UI and textures) and a release build of
// wowlua; `run.sh` runs it.
const vscode = require("vscode");
const path = require("path");
const fs = require("fs");

// The run's log: a file (`run.sh` shows it), else the console.
const LOG = process.env.WOWLUA_VSCODE_LOG;
function log(...a) {
    if (LOG) fs.appendFileSync(LOG, a.join(" ") + "\n");
    else console.log(...a);
}

function waitFor(pred, ms, what) {
    return new Promise((resolve, reject) => {
        const start = Date.now();
        const tick = () => {
            const v = pred();
            if (v) return resolve(v);
            if (Date.now() - start > ms) return reject(new Error("timed out waiting for " + what));
            setTimeout(tick, 200);
        };
        tick();
    });
}

async function scenario(name, config, check) {
    log(`== ${name}`);
    const stops = [];
    const tracker = vscode.debug.registerDebugAdapterTrackerFactory("wowlua", {
        createDebugAdapterTracker() {
            return {
                onDidSendMessage(m) {
                    if (m.type === "event" && m.event === "stopped") stops.push(m.body);
                    if (m.type === "event" && m.event === "output" && m.body.category === "console") log("console:", m.body.output.trim());
                },
                onError(e) { log("adapter error:", e.message); },
            };
        },
    });
    const folder = vscode.workspace.workspaceFolders[0];
    const ok = await vscode.debug.startDebugging(folder, config);
    log("started:", ok);
    let session;
    try {
        await waitFor(() => stops.length > 0, 120000, "a stop");
        session = vscode.debug.activeDebugSession;
        log("stopped:", JSON.stringify(stops[0]).slice(0, 200));
        // VS Code asks for the stack and opens the top frame's file itself.
        await waitFor(() => vscode.window.activeTextEditor, 20000, "an editor");
        await new Promise(r => setTimeout(r, 1500));
        const ed = vscode.window.activeTextEditor;
        const stack = await session.customRequest("stackTrace", { threadId: 1, startFrame: 0, levels: 5 });
        const frames = stack.stackFrames.map(f => `${f.name} @ ${f.source ? (f.source.path || f.source.name) : "-"}:${f.line}`);
        log("stack:", frames.join(" | "));
        log("editor:", ed.document.uri.fsPath, "line", ed.selection.active.line + 1);
        check(stops[0], ed, stack.stackFrames);
        log("PASS", name);
    } finally {
        tracker.dispose();
        if (vscode.debug.activeDebugSession) await vscode.debug.stopDebugging();
        await new Promise(r => setTimeout(r, 2000));
        await vscode.commands.executeCommand("workbench.action.closeAllEditors");
    }
}

function assert(c, msg) { if (!c) throw new Error("FAIL: " + msg); }

// The UI panel and the Frames view on the running addon: a click in the
// panel is a click in the game, stopping at the OnClick breakpoint.
async function uiScenario(widgets) {
    const name = "addon: the UI panel draws, clicks and outlines";
    log(`== ${name}`);
    const api = vscode.extensions.getExtension("wowlua.wowlua").exports;
    const folder = vscode.workspace.workspaceFolders[0];
    const stops = [];
    const tracker = vscode.debug.registerDebugAdapterTrackerFactory("wowlua", {
        createDebugAdapterTracker() {
            return { onDidSendMessage(m) { if (m.type === "event" && m.event === "stopped") stops.push(m.body); } };
        },
    });
    try {
        await vscode.debug.startDebugging(folder, { type: "wowlua", request: "launch", name: "run the addon", addon: true, flavor: "camelot" });
        const session = await waitFor(() => vscode.debug.activeDebugSession, 30000, "the session");
        await waitFor(() => api.ui.state().open && api.ui.state().shots >= 2, 60000, "the panel drawing twice");
        log("panel:", JSON.stringify(api.ui.state()));
        await session.customRequest("wowlua/input", { action: "slash", text: "/wdemo" });
        let button;
        await waitFor(() => {
            session.customRequest("wowlua/findFrames", { query: "Clicks" }).then((list) => {
                button = list.find((f) => f.type === "Button" && f.rect && f.visible);
            });
            return button;
        }, 30000, "the counter button");
        log("button:", JSON.stringify(button));
        // Opening the frame picker while the game runs (no stop yet) loads
        // the frames shown now, the demo's window among them.
        await api.ui.handleMessage({ type: "framesWanted" });
        assert(api.ui.frames.some((f) => f.label === "WidgetDemoWindow"), "the picker has no WidgetDemoWindow before a stop");
        const before = stops.length;
        // The click answers once the breakpoint it hits is continued.
        const clicked = api.ui.handleMessage({ type: "click", x: button.rect.x + button.rect.width / 2, y: button.rect.y + button.rect.height / 2, button: "LeftButton" });
        await waitFor(() => stops.length > before, 30000, "the OnClick breakpoint");
        log("stopped by the click:", JSON.stringify(stops[stops.length - 1]));
        await waitFor(() => api.ui.state().paused, 10000, "the panel paused");
        // The Frames view has the window's frames; one outlines in the panel.
        await api.frames.refresh();
        assert(api.frames.roots.length > 0, "the Frames view is empty");
        const node = api.frames.find(button.id);
        assert(node && node.rect, "the button is not in the Frames view");
        await vscode.commands.executeCommand("wowlua.outlineFrame", node);
        assert(api.ui.highlight && api.ui.highlight.width === button.rect.width, "no outline");
        if (process.env.WOWLUA_SHOT) {
            const shot = await session.customRequest("wowlua/screenshot", {});
            fs.writeFileSync(process.env.WOWLUA_SHOT, Buffer.from(shot.png, "base64"));
            log("captured", process.env.WOWLUA_SHOT, JSON.stringify(api.ui.highlight));
        }
        // Zoom: the picker lists the visible named frames; picking one zooms
        // to it, outlined; the Frames view's zoom does the same.
        assert(api.ui.state().frames > 0, "the picker has no frames");
        const win = api.ui.frames.find((f) => f.label === "WidgetDemoWindow");
        assert(win, "WidgetDemoWindow not in the picker");
        await api.ui.handleMessage({ type: "pick", label: "WidgetDemoWindow" });
        assert(api.ui.highlight === win.rect, "picking did not outline the window");
        await vscode.commands.executeCommand("wowlua.zoomToFrame", node);
        assert(api.ui.highlight === node.rect, "the view's zoom did not outline the button");
        // Zoomed in on the button, the picture keeps the screen's shape
        // (VS Code's webview styles capped its width: it was squeezed).
        const m = await waitFor(() => {
            const x = api.ui.state().metrics;
            return x && x.zoom > 1.2 ? x : undefined;
        }, 20000, "the zoomed picture's size");
        const shape = (m.width / m.height) / (1920 / 1080);
        log("zoomed:", JSON.stringify(m), "shape", shape.toFixed(3));
        assert(Math.abs(shape - 1) < 0.01, "the zoomed picture is stretched: " + JSON.stringify(m));
        // Zoomed in, the panel asks for a sharper drawing itself: two or
        // three times the pixels, the screen's size the same.
        const sharp = await waitFor(() => {
            const l = api.ui.state().last;
            return l && l.scale >= 2 ? l : undefined;
        }, 20000, "a sharper drawing");
        assert(sharp.width === 1920 * sharp.scale && sharp.height === 1080 * sharp.scale, "drawing " + JSON.stringify(sharp));
        log("zoom:", JSON.stringify(api.ui.state()));
        await vscode.commands.executeCommand("workbench.action.debug.continue");
        await clicked;
        let text;
        await waitFor(() => {
            session.customRequest("wowlua/findFrames", { query: "Clicks" }).then((list) => {
                const b = list.find((f) => f.type === "Button" && f.visible);
                text = b && b.text;
            });
            return text === "Clicks: 1";
        }, 30000, "the button's text Clicks: 1");
        log("after the click:", text, "frames roots:", api.frames.roots.length);
        log("PASS", name);
    } finally {
        tracker.dispose();
        if (vscode.debug.activeDebugSession) await vscode.debug.stopDebugging();
        await new Promise(r => setTimeout(r, 2000));
    }
}

exports.run = async function () {
    const folder = vscode.workspace.workspaceFolders[0].uri.fsPath;
    await vscode.workspace.getConfiguration("wowlua").update("path", process.env.WOWLUA_BIN, vscode.ConfigurationTarget.Global);
    const widgets = vscode.Uri.file(path.join(folder, "src/ui/widgets.wowlua"));
    vscode.debug.addBreakpoints([new vscode.SourceBreakpoint(new vscode.Location(widgets, new vscode.Position(20, 0)))]);
    const base = { type: "wowlua", request: "launch", name: "wowlua: debug the counter test", test: "tests/window.test.wowlua", filter: "clicking the counter" };
    let failed = 0;
    try {
        await scenario("camelot: the breakpoint in OnClick", { ...base, flavor: "camelot" }, (stop, ed, frames) => {
            assert(stop.reason === "breakpoint", "stopped for " + stop.reason);
            assert(ed.document.uri.fsPath === widgets.fsPath, "editor shows " + ed.document.uri.fsPath);
            assert(ed.selection.active.line + 1 === 21, "editor line " + (ed.selection.active.line + 1));
            assert(frames[0].name === "increment", "top frame " + frames[0].name);
        });
    } catch (e) { failed++; log(String(e)); }
    try {
        await scenario("standard: Blizzard's load error opens its file", { ...base, flavor: "standard" }, (stop, ed) => {
            assert(stop.reason === "exception", "stopped for " + stop.reason);
            assert(ed.document.uri.fsPath.endsWith("SharedColorConstants.lua"), "editor shows " + ed.document.uri.fsPath);
            assert(ed.selection.active.line + 1 === 23, "editor line " + (ed.selection.active.line + 1));
        });
    } catch (e) { failed++; log(String(e)); }
    try {
        await uiScenario(widgets);
    } catch (e) { failed++; log(String(e)); }
    log(failed ? `${failed} FAILED` : "ALL PASSED");
    if (failed) throw new Error(`${failed} scenarios failed`);
};
