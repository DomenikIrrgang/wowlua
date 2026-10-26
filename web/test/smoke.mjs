// The built package as a page uses it, in Node: `npm run build && npm test`.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { load, version, Playground } from "../index.js";

await load(await readFile(new URL("../pkg/wowlua_web_bg.wasm", import.meta.url)));
const p = new Playground();
console.log(`wowlua ${version()}`);

const built = p.build();
assert.equal(built.ok, true, JSON.stringify(built.diagnostics));
assert.ok(built.files.some((f) => f.path === "Playground.toc"));
assert.match(built.files.find((f) => f.path === "src/main.lua").text, /PLAYER_LOGIN/);

p.writeFile("src/main.wowlua", 'local n: number = "one"\nprint(n)\n');
const found = p.check();
assert.equal(found[0].code, "E2001");
assert.equal(found[0].file, "src/main.wowlua");

const link = p.share();
p.reset({ "wowlua.json": "{}" });
assert.equal(p.openShared(link), true);
assert.ok("src/main.wowlua" in p.files());

// the addon runs: login, frames, a click (no canvas in Node: not drawn)
p.writeFile(
    "src/main.wowlua",
    [
        'print("logged in")',
        'local b = CreateFrame("Button", nil, UIParent)',
        "b:SetSize(100, 40)",
        'b:SetPoint("CENTER")',
        'b:SetScript("OnClick", function() print("clicked") end)',
        "",
    ].join("\n"),
);
const started = p.run({ flavor: "camelot" });
assert.equal(started.ok, true, JSON.stringify(started));
assert.ok(p.running);
let printed = p.advance().map((o) => o.text);
assert.ok(printed.includes("logged in"), JSON.stringify(printed));
const [w, h] = p.screenSize();
printed = p.input({ action: "click", x: w / 2, y: h / 2 }).map((o) => o.text);
assert.deepEqual(printed, ["clicked"]);
p.stop();
assert.equal(p.running, false);

// the debugger through the page's link (the worker's mailbox stands in
// here: requests for when it waits, everything sent kept)
const requests = [
    { id: 1, command: "stackTrace" },
    { id: 2, command: "evaluate", expression: "count", frame: 1 },
    { id: 3, command: "continue" },
];
const sent = [];
globalThis.wowluaDebugRecv = (timeoutMs) => (timeoutMs < 0 ? JSON.stringify(requests.shift()) : undefined);
globalThis.wowluaDebugSend = (text) => sent.push(JSON.parse(text));
p.writeFile(
    "src/main.wowlua",
    [
        "local count = 0",
        'local b = CreateFrame("Button", nil, UIParent)',
        "b:SetSize(100, 40)",
        'b:SetPoint("CENTER")',
        'b:SetScript("OnClick", function()',
        "    count = count + 1",
        '    print("clicked", count)',
        "end)",
        "",
    ].join("\n"),
);
const debugged = p.run({ debug: { breakpoints: { "src/main.wowlua": [{ line: 7 }] } } });
assert.equal(debugged.ok, true, JSON.stringify(debugged));
p.advance();
printed = p.input({ action: "click", x: w / 2, y: h / 2 }).map((o) => o.text);
assert.deepEqual(printed, ["clicked 1"]);
assert.equal(sent.find((m) => m.event === "stopped")?.reason, "breakpoint", JSON.stringify(sent));
const top = sent.find((m) => m.id === 1).response.frames[0];
assert.deepEqual([top.source, top.line], ["src/main.wowlua", 7]);
assert.equal(sent.find((m) => m.id === 2).response.variable.value, "1");
p.stop();
assert.equal(sent.at(-1).event, "terminated");

// the language server: diagnostics of the open file, then a hover
const got = [];
const server = p.languageServer((m) => got.push(m));
const uri = "file:///playground/src/main.wowlua";
server.send({ jsonrpc: "2.0", id: 1, method: "initialize", params: { rootUri: "file:///playground", capabilities: {} } });
server.send({ jsonrpc: "2.0", method: "initialized", params: {} });
server.send({
    jsonrpc: "2.0",
    method: "textDocument/didOpen",
    params: { textDocument: { uri, languageId: "wowlua", version: 1, text: "local count = 1\nlocal n: string = count\nprint(n)\n" } },
});
const until = async (test) => {
    for (let i = 0; i < 200 && !got.some(test); i++) await new Promise((r) => setTimeout(r, 25));
    return got.find(test);
};
const published = await until((m) => m.method === "textDocument/publishDiagnostics" && m.params.uri === uri);
assert.deepEqual(published.params.diagnostics.map((d) => d.code), ["E2001"]);
server.send({ jsonrpc: "2.0", id: 2, method: "textDocument/hover", params: { textDocument: { uri }, position: { line: 0, character: 7 } } });
const hover = await until((m) => m.id === 2);
assert.match(JSON.stringify(hover.result.contents), /number/);
server.close();
console.log("ok");
