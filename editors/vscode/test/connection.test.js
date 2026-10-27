// The language client's connection to a real `wowlua lsp` (decision T368),
// without VS Code: an unsaved edit's error comes back as a diagnostic, and
// a fix clears it. `node --test editors/vscode/test/connection.test.js` runs it, with
// WOWLUA_BIN naming the wowlua binary (target/debug/wowlua by default).
const { test } = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { pathToFileURL } = require("url");
const { Connection } = require("../connection");

const repo = path.resolve(__dirname, "../../..");
const bin = process.env.WOWLUA_BIN || path.join(repo, "target/debug/wowlua");

// Resolves with the first diagnostics for `uri` that `accept` takes.
function diagnosticsFor(connection, uri, accept) {
    return new Promise((resolve) => {
        connection.onNotification("textDocument/publishDiagnostics", (p) => {
            if (p.uri === uri && accept(p.diagnostics)) {
                resolve(p.diagnostics);
            }
        });
    });
}

test("an unsaved edit is checked, and a fix clears it", { skip: !fs.existsSync(bin) && `no ${bin}` }, async () => {
    const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "wowlua-lsp-")));
    fs.writeFileSync(
        path.join(dir, "wowlua.json"),
        JSON.stringify({ package: { name: "lsp-client", version: "0.1.0", wowlua: "^0.13" }, flavors: { camelot: {} } }),
    );
    fs.mkdirSync(path.join(dir, "src"));
    const file = path.join(dir, "src", "main.wowlua");
    fs.writeFileSync(file, "print(1)\n");
    const uri = pathToFileURL(file).toString();
    const connection = new Connection(bin, dir);
    try {
        const init = await connection.request("initialize", { processId: process.pid, capabilities: {} });
        assert.strictEqual(init.capabilities.documentFormattingProvider, true);
        connection.notify("initialized", {});
        const broken = diagnosticsFor(connection, uri, (d) => d.length > 0);
        connection.notify("textDocument/didOpen", {
            textDocument: { uri, languageId: "wowlua", version: 1, text: 'local x: number = "one"\nprint(x)\n' },
        });
        const found = await broken;
        assert.ok(found.some((d) => d.code === "E2001" && d.range.start.line === 0), JSON.stringify(found));
        const cleared = diagnosticsFor(connection, uri, (d) => d.length === 0);
        connection.notify("textDocument/didChange", {
            textDocument: { uri, version: 2 },
            contentChanges: [{ text: "local x: number = 1\nprint(x)\n" }],
        });
        await cleared;
        const edits = await connection.request("textDocument/formatting", {
            textDocument: { uri },
            options: { tabSize: 4, insertSpaces: false },
        });
        assert.deepStrictEqual(edits, []);
        assert.strictEqual(fs.readFileSync(file, "utf8"), "print(1)\n");
    } finally {
        await connection.stop();
        fs.rmSync(dir, { recursive: true, force: true });
    }
});
