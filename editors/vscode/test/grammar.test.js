// The TextMate grammar (decision T371), tokenized as VS Code tokenizes it:
// with VS Code's own `vscode-textmate` and `vscode-oniguruma`, read from
// the installed app (skipped without one). `node --test
// editors/vscode/test/grammar.test.js` runs it; VSCODE_APP names another
// app folder.
const { test } = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const os = require("os");
const path = require("path");

const app = process.env.VSCODE_APP || "/Applications/Visual Studio Code.app/Contents/Resources/app";
const asar = path.join(app, "node_modules.asar");
const wasm = path.join(app, "node_modules.asar.unpacked/vscode-oniguruma/release/onig.wasm");

// A file of the app's `node_modules.asar` (a JSON header, then the files).
function fromAsar(file) {
    const fd = fs.openSync(asar, "r");
    try {
        const head = Buffer.alloc(16);
        fs.readSync(fd, head, 0, 16, 0);
        const base = 8 + head.readUInt32LE(4);
        const json = Buffer.alloc(head.readUInt32LE(12));
        fs.readSync(fd, json, 0, json.length, 16);
        let node = JSON.parse(json.toString());
        for (const part of file.split("/")) node = node.files[part];
        const out = Buffer.alloc(node.size);
        fs.readSync(fd, out, 0, node.size, base + Number(node.offset));
        return out;
    } finally {
        fs.closeSync(fd);
    }
}

// VS Code's tokenizer, loaded from the app.
async function grammar() {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wowlua-tm-"));
    for (const m of ["vscode-textmate", "vscode-oniguruma"]) {
        fs.mkdirSync(path.join(dir, m, "release"), { recursive: true });
        fs.writeFileSync(path.join(dir, m, "release/main.js"), fromAsar(`${m}/release/main.js`));
    }
    const tm = require(path.join(dir, "vscode-textmate/release/main.js"));
    const onig = require(path.join(dir, "vscode-oniguruma/release/main.js"));
    await onig.loadWASM(fs.readFileSync(wasm).buffer);
    const registry = new tm.Registry({
        onigLib: Promise.resolve({
            createOnigScanner: (s) => new onig.OnigScanner(s),
            createOnigString: (s) => new onig.OnigString(s),
        }),
        loadGrammar: async () =>
            tm.parseRawGrammar(
                fs.readFileSync(path.join(__dirname, "../syntaxes/wowlua.tmLanguage.json"), "utf8"),
                "wowlua.tmLanguage.json",
            ),
    });
    return { g: await registry.loadGrammar("source.wowlua"), tm };
}

module.exports = { grammar };

const SAMPLE = `--- A button that counts.
import Ledger, format as fmt from "./money"
import * as mock from "wowlua:mock"

@script("OnClick")
export class Counter extends Button
    private count: number = 0
    get total(): number
        return self.count
    end
    constructor(parent: Frame?)
        super(nil, parent)
    end
end

type Point = { x: number, y: number }
const LIMIT = 10
local function bump(by: number | nil): string
    self:refresh()
    local t = type(by)
    local n = (by :: number) ?? 1
    n += 0x1F
    return \`count {n + 1}\`
end
--[[ a block ]] local s = [[long]]
for i = 1, 3 do continue end
local v = a ?: b
interface Opts
    size?: number
end
table.sort(list, function(a: Made, b: Made): boolean return a.seq < b.seq end)
local after = 1
export abstract class Bar<B extends Button> extends BarBase implements Shown, Sized
end
export type Summary = {
export const MAX = 3
`;

// The scopes of the first token whose text is `text` on line `line` (0-based).
function scopes(tokenized, line, text) {
    const found = tokenized[line].find((t) => t.text === text);
    assert.ok(found, `no token "${text}" on line ${line}: ${tokenized[line].map((t) => t.text).join("|")}`);
    return found.scopes.join(" ");
}

test("wowlua code is colored by what it is", { skip: !fs.existsSync(asar) && "no VS Code app" }, async () => {
    const { g, tm } = await grammar();
    let state = tm.INITIAL;
    const tokenized = SAMPLE.split("\n").map((l) => {
        const r = g.tokenizeLine(l, state);
        state = r.ruleStack;
        return r.tokens.map((t) => ({ text: l.slice(t.startIndex, t.endIndex), scopes: t.scopes }));
    });
    const has = (line, text, scope) => {
        const s = scopes(tokenized, line, text);
        assert.ok(s.includes(scope), `"${text}" (line ${line}) is ${s}, not ${scope}`);
    };
    has(0, " A button that counts.", "comment.line.documentation");
    has(1, "import", "keyword.control.import");
    has(1, "from", "keyword.control.import");
    has(1, "\"", "string.quoted.double");
    has(2, "*", "keyword.operator");
    has(4, "script", "entity.name.function.decorator");
    has(5, "export", "storage.modifier.export");
    has(5, "class", "storage.type");
    has(5, "Counter", "entity.name.type");
    has(5, "extends", "storage.modifier");
    has(6, "private", "storage.modifier");
    has(6, "number", "support.type.primitive");
    has(7, "get", "storage.type.accessor");
    has(7, "total", "entity.name.function");
    has(8, "self", "variable.language.self");
    has(10, "constructor", "storage.type.constructor");
    has(10, "Frame", "entity.name.type");
    has(11, "super", "variable.language.super");
    has(15, "type", "storage.type");
    has(15, "Point", "entity.name.type.alias");
    has(16, "LIMIT", "variable.other.constant");
    has(17, "bump", "entity.name.function");
    has(17, "string", "support.type.primitive");
    // a method call is no annotation
    has(18, "refresh", "entity.name.function.method");
    // `type(x)` stays a call
    has(19, "type", "entity.name.function.call");
    has(20, "::", "keyword.operator.cast");
    has(20, "??", "keyword.operator.optional");
    has(21, "+=", "keyword.operator.assignment.compound");
    has(21, "0x1F", "constant.numeric.hex");
    has(22, "count ", "string.interpolated");
    has(22, "n", "meta.embedded.expression");
    has(24, " a block ", "comment.block");
    has(24, "long", "string.quoted.other.multiline");
    has(25, "continue", "keyword.control");
    has(25, "do", "keyword.control");
    // \`?:\` the operator, and \`?:\` an optional member's annotation
    has(26, "?:", "keyword.operator.optional");
    has(26, "b", "variable.other.readwrite");
    has(28, "number", "support.type.primitive");
    // a return type ends where the body starts on the same line, and
    // nothing spills into the next line
    has(30, "boolean", "support.type.primitive");
    has(30, "return", "keyword.control");
    has(30, "<", "keyword.operator.comparison");
    has(31, "local", "storage.modifier.local");
    // a type parameter's constraint, the base class and interfaces
    has(32, "extends", "storage.modifier");
    has(32, "BarBase", "entity.other.inherited-class");
    has(32, "Shown", "entity.other.inherited-class");
    has(32, "Sized", "entity.other.inherited-class");
    // after `export` too
    has(34, "export", "storage.modifier.export");
    has(34, "type", "storage.type");
    has(34, "Summary", "entity.name.type.alias");
    has(35, "const", "storage.modifier");
    has(35, "MAX", "variable.other.constant");
    assert.ok(scopes(tokenized, 32, "B").includes("entity.name.type"));
    assert.ok(tokenized[32].filter((t) => t.text === "extends").every((t) => t.scopes.join(" ").includes("storage.modifier")));
});
