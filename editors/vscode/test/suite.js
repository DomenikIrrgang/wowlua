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
        // Opened, the picker stays open as the frames come, listing them
        // (a datalist closed when its options were renewed).
        api.ui.picker = undefined;
        api.ui.post({ type: "openPicker" });
        const picker = await waitFor(() => api.ui.state().picker, 20000, "the picker's state");
        log("picker:", JSON.stringify(picker));
        assert(picker.open && picker.items > 0, "the picker closed or is empty: " + JSON.stringify(picker));
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
        }, 20000, "the zoomed picture's size").catch((e) => { throw new Error(e.message + ": " + JSON.stringify(api.ui.state())); });
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

// The addon's session as a user runs it: the world profile's game, a
// system command typed in the Debug Console, its answer in the chat frame
// the panel draws, the panel drawing on while the game runs.
async function chatScenario() {
    const name = "addon: a command's answer shows in the panel's chat";
    log(`== ${name}`);
    const api = await vscode.extensions.getExtension("wowlua.wowlua").activate();
    const folder = vscode.workspace.workspaceFolders[0];
    try {
        await vscode.debug.startDebugging(folder, { type: "wowlua", request: "launch", name: "run the addon", addon: true, flavor: "camelot", profile: "camelot" });
        const session = await waitFor(() => vscode.debug.activeDebugSession, 30000, "the session");
        await waitFor(() => api.ui.state().open && api.ui.state().shots >= 2, 60000, "the panel drawing twice");
        await new Promise(r => setTimeout(r, 3000));
        await session.customRequest("evaluate", { expression: "/played", context: "repl" });
        const at = api.ui.state().shots;
        await waitFor(() => api.ui.state().shots >= at + 4, 20000, "the panel drawing on");
        log("panel:", JSON.stringify(api.ui.state()));
        // Closed and opened again, the panel draws on while the game runs.
        api.ui.panel.dispose();
        await vscode.commands.executeCommand("wowlua.showUI");
        const reopened = api.ui.state().shots;
        await waitFor(() => api.ui.state().shots >= reopened + 3, 20000, "the reopened panel drawing on");
        if (process.env.WOWLUA_CHAT_SHOT) {
            fs.writeFileSync(process.env.WOWLUA_CHAT_SHOT, Buffer.from(api.ui.lastPng, "base64"));
            log("captured", process.env.WOWLUA_CHAT_SHOT);
        }
        log("PASS", name);
    } finally {
        if (vscode.debug.activeDebugSession) await vscode.debug.stopDebugging();
        await new Promise(r => setTimeout(r, 2000));
    }
}

// The language client (decision T368): an unsaved edit's type error shows
// in the Problems view and goes again when undone; the outline lists the
// file's functions; go to definition, hover and completion find the
// method; Format Document formats it.
async function lspScenario(widgets) {
    const name = "the language server checks an unsaved edit";
    log(`== ${name}`);
    const doc = await vscode.workspace.openTextDocument(widgets);
    const ed = await vscode.window.showTextDocument(doc);
    const errors = () => vscode.languages.getDiagnostics(widgets).filter(d => d.code === "E2001");
    try {
        await ed.edit(b => b.insert(new vscode.Position(0, 0), "local broken: number = \"one\"\n"));
        const found = await waitFor(() => errors().length > 0 && errors(), 60000, "the edit's E2001");
        assert(found[0].range.start.line === 0, "E2001 on line " + found[0].range.start.line);
        log("diagnostic:", found[0].message.split("\n")[0]);
        await ed.edit(b => b.delete(new vscode.Range(0, 0, 1, 0)));
        await waitFor(() => errors().length === 0, 60000, "the E2001 cleared");
        const symbols = await vscode.commands.executeCommand("vscode.executeDocumentSymbolProvider", widgets);
        const names = (symbols || []).map(s => s.name);
        log("outline:", names.join(", "));
        assert(names.length > 0, "no outline");
        // `self:refresh()` in the constructor: its method below.
        const text = doc.getText();
        const call = doc.positionAt(text.indexOf("self:refresh()") + "self:".length + 1);
        const definitions = await vscode.commands.executeCommand("vscode.executeDefinitionProvider", widgets, call);
        const target = (definitions || [])[0];
        assert(target, "no definition of refresh");
        const range = target.targetRange || target.range;
        const uri = target.targetUri || target.uri;
        const line = doc.lineAt(range.start.line).text;
        log("definition:", uri.fsPath === widgets.fsPath ? "same file" : uri.fsPath, "line", range.start.line + 1, line.trim());
        assert(uri.fsPath === widgets.fsPath && line.includes("function refresh"), "definition at " + line);
        // hovering `refresh` shows its type
        const hovers = await vscode.commands.executeCommand("vscode.executeHoverProvider", widgets, call);
        const shown = (hovers || []).flatMap((h) => h.contents.map((c) => c.value || String(c))).join("\n");
        log("hover:", shown.replace(/\n/g, " "));
        assert(shown.includes("refresh: "), "hover shows " + shown);
        // completing after `self:` lists the class's methods, `refresh` too
        const colon = doc.positionAt(text.indexOf("self:refresh()") + "self:".length);
        const completions = await vscode.commands.executeCommand("vscode.executeCompletionItemProvider", widgets, colon, ":");
        const offered = ((completions && completions.items) || []).map((i) => (typeof i.label === "string" ? i.label : i.label.label));
        log("completion:", offered.slice(0, 12).join(", "));
        assert(offered.includes("refresh"), "completion offers " + offered.join(", "));
        const edits = await vscode.commands.executeCommand("vscode.executeFormatDocumentProvider", widgets, { tabSize: 4, insertSpaces: false });
        assert(Array.isArray(edits), "formatting gave " + edits);
        log("PASS", name);
    } finally {
        await vscode.commands.executeCommand("workbench.action.revertAndCloseActiveEditor");
    }
}

// Completion as a user gets it (decision T377): typing a name opens the
// suggestions by itself, after `.` too, and accepting one inserts it.
async function completeScenario(widgets) {
    const name = "completion suggests as you type";
    log(`== ${name}`);
    const doc = await vscode.workspace.openTextDocument(widgets);
    const ed = await vscode.window.showTextDocument(doc);
    // (inside a method: where one sets a field)
    const lines = doc.getText().split("\n");
    const line = lines.findIndex((l) => /^\s+self\.\w+ = /.test(l));
    const field = lines[line].match(/self\.(\w+)/)[1];
    try {
        // (the first check done)
        await waitFor(() => vscode.languages.getDiagnostics(widgets) !== undefined, 1000, "diagnostics");
        await new Promise((r) => setTimeout(r, Number(process.env.WOWLUA_SETTLE || 4000)));
        const typed = async (text, expect, pace = 60) => {
            await ed.edit((b) => b.insert(new vscode.Position(line, 0), "        \n"));
            ed.selection = new vscode.Selection(line, 8, line, 8);
            for (const ch of text) {
                await vscode.commands.executeCommand("type", { text: ch });
                await new Promise((r) => setTimeout(r, pace));
            }
            await new Promise((r) => setTimeout(r, 3000));
            await vscode.commands.executeCommand("acceptSelectedSuggestion");
            await new Promise((r) => setTimeout(r, 300));
            const got = doc.lineAt(line).text.trim();
            log(`typed ${JSON.stringify(text)}: ${JSON.stringify(got)}`);
            await ed.edit((b) => b.delete(new vscode.Range(line, 0, line + 1, 0)));
            return got === expect;
        };
        const results = [
            await typed("local test = C_Tim", "local test = C_Timer"),
            await typed("self." + field.slice(0, 3), "self." + field),
            await typed("local n = C_Timer.Aft", "local n = C_Timer.After"),
            // (the quote closes itself: the name lands between the quotes)
            await typed('local e = CreateFrame("Frame"):RegisterEvent("PLAYER_LOGI', 'local e = CreateFrame("Frame"):RegisterEvent("PLAYER_LOGIN")'),
        ];
        assert(results.every(Boolean), "results " + results.join(", "));
        // inside a call's parentheses: its parameters, the one at the cursor
        await ed.edit((b) => b.insert(new vscode.Position(line, 0), "        UnitHealth(\"player\", )\n"));
        const inCall = new vscode.Position(line, "        UnitHealth(\"player\", ".length);
        const help = await vscode.commands.executeCommand("vscode.executeSignatureHelpProvider", doc.uri, inCall, ",");
        await ed.edit((b) => b.delete(new vscode.Range(line, 0, line + 1, 0)));
        const label = help && help.signatures[help.activeSignature].label;
        log(`signature: ${label} (parameter ${help && help.activeParameter})`);
        assert(label && label.startsWith("UnitHealth(unit: string") && help.activeParameter === 1, "signature help " + label);
        log("PASS", name);
    } finally {
        await vscode.commands.executeCommand("workbench.action.revertAndCloseActiveEditor");
    }
}

// The test explorer (decision T382), on the fixture package
// `test/fixture-tests`: the file's tests found, a run of the file, a run of
// one test.
async function testsScenario() {
    const name = "the test explorer finds and runs tests";
    log(`== ${name}`);
    const api = await vscode.extensions.getExtension("wowlua.wowlua").activate();
    const folder = vscode.workspace.workspaceFolders[0].uri.fsPath;
    const uri = vscode.Uri.file(path.join(folder, "tests/math.test.wowlua"));
    // (the language server reads the file once it runs)
    for (let tries = 0; ; tries++) {
        await api.tests.discover();
        const f = api.tests.ctrl.items.get(uri.toString());
        if (f && f.children.size > 0) break;
        if (tries > 60) throw new Error("timed out waiting for the file's tests");
        await new Promise((r) => setTimeout(r, 1000));
    }
    const file = api.tests.ctrl.items.get(uri.toString());
    const names = [];
    file.children.forEach((c) => names.push(c.label));
    log("found:", names.join(", "));
    assert(["adds two numbers", "fails on purpose", "waits", "sums"].every((n) => names.includes(n)), "found " + names.join(", "));
    const token = new vscode.CancellationTokenSource().token;
    await api.tests.run(new vscode.TestRunRequest([file]), token);
    const states = Object.fromEntries(api.tests.last.map((r) => [r.name, r.state]));
    log("ran the file:", JSON.stringify(states));
    assert(states["adds two numbers"] === "passed", "adds: " + states["adds two numbers"]);
    assert(states["fails on purpose"] === "failed", "fails: " + states["fails on purpose"]);
    assert(states["waits"] === "skipped", "waits: " + states["waits"]);
    assert(states["sums [2, 3, 5]"] === "passed", "a case: " + states["sums [2, 3, 5]"]);
    const one = file.children.get(`${uri.toString()}::adds two numbers`);
    await api.tests.run(new vscode.TestRunRequest([one]), token);
    log("ran one test:", JSON.stringify(api.tests.last));
    assert(api.tests.last.length === 1 && api.tests.last[0].state === "passed", "one test: " + JSON.stringify(api.tests.last));
    // the code lenses: run and debug at the top and above each test; a
    // test's runs it
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(uri));
    const lenses = await vscode.commands.executeCommand("vscode.executeCodeLensProvider", uri, 10);
    const titles = (lenses || []).map((l) => l.command && l.command.title);
    log("lenses:", titles.length, titles.slice(0, 4).join(" | "));
    assert(titles.filter((t) => t && t.includes("Run test")).length === 4, "lenses " + titles.join(" | "));
    const sums = lenses.find((l) => l.command && l.command.title.includes("Run test") && String(l.command.arguments[0]).endsWith("::sums"));
    await vscode.commands.executeCommand(sums.command.command, ...sums.command.arguments);
    log("ran by its lens:", JSON.stringify(api.tests.last));
    assert(api.tests.last.length === 2 && api.tests.last.every((r) => r.state === "passed"), "the sums lens ran " + JSON.stringify(api.tests.last));
    // with coverage: the addon's file, its lines as the run covered them
    await api.tests.run(new vscode.TestRunRequest([file]), token, { coverage: true });
    log("coverage:", JSON.stringify(api.tests.lastCoverage));
    const main = (api.tests.lastCoverage || []).find((c) => c.file === "src/main.wowlua");
    assert(main && main.lines === 3 && main.of === 3, "coverage " + JSON.stringify(api.tests.lastCoverage));
    const details = api.tests.covered.get(vscode.Uri.file(path.join(folder, "src/main.wowlua")).toString()) || [];
    assert(details.some((d) => d instanceof vscode.DeclarationCoverage && d.name === "add" && d.executed === 4), "details " + details.length);
    log("PASS", name);
}

exports.run = async function () {
    const folder = vscode.workspace.workspaceFolders[0].uri.fsPath;
    await vscode.workspace.getConfiguration("wowlua").update("path", process.env.WOWLUA_BIN, vscode.ConfigurationTarget.Global);
    const widgets = vscode.Uri.file(path.join(folder, "src/ui/widgets.wowlua"));
    vscode.debug.addBreakpoints([new vscode.SourceBreakpoint(new vscode.Location(widgets, new vscode.Position(20, 0)))]);
    const base = { type: "wowlua", request: "launch", name: "wowlua: debug the counter test", test: "tests/window.test.wowlua", filter: "clicking the counter" };
    let failed = 0;
    if (process.env.WOWLUA_SCENARIO === "lsp") {
        try { await lspScenario(widgets); } catch (e) { failed++; log(String(e)); }
        log(failed ? `${failed} FAILED` : "ALL PASSED");
        if (failed) throw new Error(`${failed} scenarios failed`);
        return;
    }
    if (process.env.WOWLUA_SCENARIO === "tests") {
        try { await testsScenario(); } catch (e) { failed++; log(String(e)); }
        log(failed ? `${failed} FAILED` : "ALL PASSED");
        if (failed) throw new Error(`${failed} scenarios failed`);
        return;
    }
    if (process.env.WOWLUA_SCENARIO === "complete") {
        // (WOWLUA_COMPLETE_FILE: another package's file, its folder open)
        const file = process.env.WOWLUA_COMPLETE_FILE ? vscode.Uri.file(process.env.WOWLUA_COMPLETE_FILE) : widgets;
        try { await completeScenario(file); } catch (e) { failed++; log(String(e)); }
        log(failed ? `${failed} FAILED` : "ALL PASSED");
        if (failed) throw new Error(`${failed} scenarios failed`);
        return;
    }
    if (process.env.WOWLUA_SCENARIO === "chat") {
        try { await chatScenario(); } catch (e) { failed++; log(String(e)); }
        log(failed ? `${failed} FAILED` : "ALL PASSED");
        if (failed) throw new Error(`${failed} scenarios failed`);
        return;
    }
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
    try {
        await chatScenario();
    } catch (e) { failed++; log(String(e)); }
    try {
        await lspScenario(widgets);
    } catch (e) { failed++; log(String(e)); }
    log(failed ? `${failed} FAILED` : "ALL PASSED");
    if (failed) throw new Error(`${failed} scenarios failed`);
};
