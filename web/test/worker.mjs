// The worker and its client as a page uses them, in Node: worker.js runs
// on this thread behind a stand-in Worker (messages delivered as a
// browser does, one task each), client.js talks to it.
// `npm run build && npm test`.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { connect } from "../client.js";

// The worker's side: its global scope's `postMessage` and `onmessage`.
const page = new Set();
globalThis.onmessage = null;
globalThis.postMessage = (data) => setTimeout(() => { for (const listen of page) listen({ data }); });
await import("../worker.js");
const worker = {
    addEventListener(type, listen) {
        if (type === "message") page.add(listen);
    },
    postMessage(data) {
        setTimeout(() => globalThis.onmessage({ data }));
    },
    terminate() {},
};
worker.postMessage({ wasm: await readFile(new URL("../pkg/wowlua_web_bg.wasm", import.meta.url)) });
const wowlua = connect(worker);

const until = async (what, test) => {
    for (let i = 0; i < 400; i++) {
        const found = test();
        if (found) return found;
        await new Promise((r) => setTimeout(r, 25));
    }
    assert.fail(`${what}: never came`);
};

// the language server has the file open with the editor's text
const got = [];
wowlua.lsp.onMessage((m) => got.push(m));
const uri = "file:///playground/src/main.wowlua";
wowlua.lsp.send({ jsonrpc: "2.0", id: 1, method: "initialize", params: { rootUri: "file:///playground", capabilities: {} } });
wowlua.lsp.send({ jsonrpc: "2.0", method: "initialized", params: {} });
wowlua.lsp.send({
    jsonrpc: "2.0",
    method: "textDocument/didOpen",
    params: { textDocument: { uri, languageId: "wowlua", version: 1, text: "local a = 1\n" } },
});
const published = (file) => got.filter((m) => m.method === "textDocument/publishDiagnostics" && m.params.uri === file).at(-1);
await until("diagnostics", () => published(uri));
assert.deepEqual(published(uri).params.diagnostics.map((d) => d.code), ["W1003"]);

// what the page writes is the package: check reads it, not the editor's text
await wowlua.call("writeFile", "src/main.wowlua", 'local n: number = "one"\nprint(n)\n');
const found = await wowlua.call("check");
assert.deepEqual(found.map((d) => d.code), ["E2001"]);
assert.match(found[0].text, /^error\[E2001\]/);
assert.match(found[0].text, /src\/main\.wowlua:1:7/);

// and the server checks the files the page writes
const util = "file:///playground/src/util.wowlua";
await wowlua.call("writeFile", "src/util.wowlua", "local s: string = 2\nprint(s)\n");
await until("the written file's diagnostics", () => published(util));
assert.deepEqual(published(util).params.diagnostics.map((d) => d.code), ["E2001"]);
await wowlua.call("deleteFile", "src/util.wowlua");

// the CLI's lints: W2063 is off until the manifest turns it on
await wowlua.call("writeFile", "src/main.wowlua", "local function show(value)\n    print(value)\nend\nshow(1)\n");
assert.deepEqual(await wowlua.call("check"), []);

// a run without a canvas: its output comes to onOutput, and `advance`
const outputs = [];
wowlua.onOutput((o) => outputs.push(o.text));
await wowlua.call("writeFile", "src/main.wowlua", 'print("hello")\nC_Timer.After(0.2, function() print("later") end)\n');
const started = await wowlua.call("run");
assert.equal(started.ok, true, JSON.stringify(started));
await until("the run's print", () => outputs.includes("hello"));
await until("the timer's print", () => outputs.includes("later"));
assert.deepEqual(await wowlua.call("advance"), []);
assert.deepEqual((await wowlua.call("world", 'print("from the world")')).map((o) => o.text), ["from the world"]);
await until("the world's print", () => outputs.includes("from the world"));
await wowlua.call("stop");

console.log("ok");
process.exit(0);
