// @wowlua/web's Web Worker: the playground off the page's thread, so a
// check or a game frame never blocks typing. Start it as a module worker
// and talk to it with `connect` (client.js), or post these messages
// yourself:
//
//   { id, call: "<method>", args: [...] }  → { id, result } or { id, error }
//       methods: files, reset, writeFile, readFile, deleteFile, check,
//                build, format, share, openShared, version,
//                loadBlizzardUI, run, stop, advance, input, screenSize,
//                profiles, profileSchema, world,
//                snapshot (the game's screen as a PNG Blob),
//                useInstallation(source, { flavor }) (the user's WoW folder:
//                a FileSystemDirectoryHandle, or a folder input's files)
//   { lsp: <JSON-RPC message> }            → { lsp: <JSON-RPC message> } …
//   { id, canvas: <OffscreenCanvas> }      → { id, result: "<backend>" };
//       the game is drawn there each frame from then on
//   { resize: [width, height] }            the canvas's new size
//   (from the worker) { output: { kind, text, stack } }: what the game
//       printed and raised, with a canvas or without (the game moves each
//       frame either way; `advance`, `input` and `world` return theirs
//       too); { progress: [done, total] } while loading Blizzard's UI
//   { debugMailbox: <SharedArrayBuffer> }  the debugger's requests come
//       there (client.js writes them; a cross-origin isolated page only):
//       stopped, the game waits on it. Its answers and events come back as
//       { debug: { id, response } | { id, error } | { event, … } }
//
// The first message may be { wasm: <url> } to load the module from there.

import { load, version, createScreen, Playground } from "./index.js";
import { Installation } from "./installation.js";

const CALLS = new Set([
    "files", "reset", "writeFile", "readFile", "deleteFile",
    "check", "build", "format", "share", "openShared",
    "run", "stop", "advance", "input", "screenSize", "profiles", "profileSchema", "world",
]);

// (what the game prints and raises, each call or frame that moved it)
const post = (output) => postMessage({ output });

let playground = null;
let server = null;
let ready = null;
let screen = null;
let canvas = null;
// The game's frames: not drawn until a canvas comes, its output posted
// all the same.
let frames = null;

// The debugger's mailbox: [state (0 empty, 1 full), length], then the
// request's UTF-8 bytes.
let mailbox = null;
const decoder = new TextDecoder();
globalThis.wowluaDebugRecv = (timeoutMs) => {
    if (!mailbox) return undefined;
    const { control, bytes } = mailbox;
    if (Atomics.load(control, 0) === 0) {
        if (timeoutMs === 0) return undefined;
        Atomics.wait(control, 0, 0, timeoutMs < 0 ? undefined : timeoutMs);
        if (Atomics.load(control, 0) === 0) return undefined;
    }
    const text = decoder.decode(bytes.slice(0, Atomics.load(control, 1)));
    Atomics.store(control, 0, 0);
    Atomics.notify(control, 0);
    return text;
};
globalThis.wowluaDebugSend = (text) => postMessage({ debug: JSON.parse(text) });

function start(wasm) {
    ready ??= load(wasm).then(() => {
        playground = new Playground();
        server = playground.languageServer((message) => postMessage({ lsp: message }));
        frames = playground.play(null, post);
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
    if (m.debugMailbox !== undefined) {
        mailbox = {
            control: new Int32Array(m.debugMailbox, 0, 2),
            bytes: new Uint8Array(m.debugMailbox, 8),
        };
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
            frames.stop();
            frames = playground.play(screen, post);
            result = screen.backend;
        } else if (m.call === "snapshot") {
            // drawn off the canvas and read back (`snapshot(scale)`)
            if (!screen) throw new Error("no canvas attached");
            const png = await screen.snapshot(playground.raw, m.args?.[0] ?? 1);
            result = new Blob([png], { type: "image/png" });
        } else if (m.call === "useInstallation") {
            const [source, { flavor = "camelot" } = {}] = m.args ?? [];
            const install = typeof source?.entries === "function" && source.kind === "directory"
                ? await Installation.fromHandle(source)
                : Installation.fromFiles(source);
            install.expose();
            result = { ...JSON.parse(playground.raw.useInstallation(flavor)), files: install.files.size };
        } else if (m.call === "version") {
            result = version();
        } else if (m.call === "loadBlizzardUI") {
            const options = { ...(m.args?.[0] ?? {}), onProgress: (done, total) => postMessage({ progress: [done, total] }) };
            const loaded = await playground.loadBlizzardUI(options);
            result = { flavor: loaded.flavor, commit: loaded.commit };
        } else if (m.call === "run" && m.args?.[0]?.debug && !mailbox) {
            throw new Error("debugging needs a cross-origin isolated page (COOP and COEP headers)");
        } else if (CALLS.has(m.call)) {
            result = playground[m.call](...(m.args ?? []));
            // (what a call made the game print goes where the frames' does)
            if (m.call === "advance" || m.call === "input" || m.call === "world") for (const output of result) post(output);
        } else {
            throw new Error(`no method ${m.call}`);
        }
        postMessage({ id: m.id, result });
    } catch (error) {
        postMessage({ id: m.id, error: String(error?.message ?? error) });
    }
};
