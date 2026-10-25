// @wowlua/web's Web Worker: the playground off the page's thread, so a
// check never blocks typing. Start it as a module worker and talk to it
// with `connect` (client.js), or post these messages yourself:
//
//   { id, call: "<method>", args: [...] }  → { id, result } or { id, error }
//       methods: files, reset, writeFile, readFile, deleteFile, check,
//                build, format, share, openShared, version
//   { lsp: <JSON-RPC message> }            → { lsp: <JSON-RPC message> } …
//
// The first message may be { wasm: <url> } to load the module from there.

import { load, version, Playground } from "./index.js";

const CALLS = new Set([
    "files", "reset", "writeFile", "readFile", "deleteFile",
    "check", "build", "format", "share", "openShared",
]);

let playground = null;
let server = null;
let ready = null;

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
    try {
        if (m.call !== "version" && !CALLS.has(m.call)) throw new Error(`no method ${m.call}`);
        const result =
            m.call === "version" ? version() : playground[m.call](...(m.args ?? []));
        postMessage({ id: m.id, result });
    } catch (error) {
        postMessage({ id: m.id, error: String(error?.message ?? error) });
    }
};
