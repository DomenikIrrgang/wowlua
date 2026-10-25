// @wowlua/web's Web Worker: the playground off the page's thread, so a
// check or a game frame never blocks typing. Start it as a module worker
// and talk to it with `connect` (client.js), or post these messages
// yourself:
//
//   { id, call: "<method>", args: [...] }  → { id, result } or { id, error }
//       methods: files, reset, writeFile, readFile, deleteFile, check,
//                build, format, share, openShared, version,
//                loadBlizzardUI, run, stop, input, screenSize,
//                snapshot (the game's screen as a PNG Blob)
//   { lsp: <JSON-RPC message> }            → { lsp: <JSON-RPC message> } …
//   { id, canvas: <OffscreenCanvas> }      → { id, result: "<backend>" };
//       the game is drawn there each frame from then on
//   { resize: [width, height] }            the canvas's new size
//   (from the worker) { output: { kind, text, stack } }: what the game
//       printed and raised; { progress: [done, total] } while loading
//       Blizzard's UI
//
// The first message may be { wasm: <url> } to load the module from there.

import { load, version, createScreen, Playground } from "./index.js";

const CALLS = new Set([
    "files", "reset", "writeFile", "readFile", "deleteFile",
    "check", "build", "format", "share", "openShared",
    "run", "stop", "input", "screenSize",
]);

let playground = null;
let server = null;
let ready = null;
let screen = null;
let canvas = null;

function start(wasm) {
    ready ??= load(wasm).then(() => {
        playground = new Playground();
        server = playground.languageServer((message) => postMessage({ lsp: message }));
    });
    return ready;
}

onmessage = async (event) => {
    const m = event.data;
    if (m.wasm !== undefined) {
        await start(m.wasm);
        return;
    }
    await start();
    if (m.lsp !== undefined) {
        server.send(m.lsp);
        return;
    }
    if (m.resize !== undefined) {
        screen?.resize(m.resize[0], m.resize[1]);
        return;
    }
    try {
        let result;
        if (m.canvas !== undefined) {
            canvas = m.canvas;
            screen = await createScreen(canvas);
            playground.play(screen, (output) => postMessage({ output }));
            result = screen.backend;
        } else if (m.call === "snapshot") {
            // drawn off the canvas and read back (`snapshot(scale)`)
            if (!screen) throw new Error("no canvas attached");
            const png = await screen.snapshot(playground.raw, m.args?.[0] ?? 1);
            result = new Blob([png], { type: "image/png" });
        } else if (m.call === "version") {
            result = version();
        } else if (m.call === "loadBlizzardUI") {
            const options = { ...(m.args?.[0] ?? {}), onProgress: (done, total) => postMessage({ progress: [done, total] }) };
            const loaded = await playground.loadBlizzardUI(options);
            result = { flavor: loaded.flavor, commit: loaded.commit };
        } else if (CALLS.has(m.call)) {
            result = playground[m.call](...(m.args ?? []));
            // (what input made the game print goes where the frames' does)
            if (m.call === "input") for (const output of result) postMessage({ output });
        } else {
            throw new Error(`no method ${m.call}`);
        }
        postMessage({ id: m.id, result });
    } catch (error) {
        postMessage({ id: m.id, error: String(error?.message ?? error) });
    }
};
