// The test explorer (decision T382): VS Code's Testing view lists each
// package's test files (`*.test.wowlua`) with the tests the language server
// reads from them (`wowlua/tests`), runs them with `wowlua test --reporter
// json` (one test exactly as `file::name`) and shows each result at its
// test, a failure at its line with the flavor it failed on. Debug runs
// `wowlua debug` on the test's file, filtered to it. Code lenses above each
// test and at the top of a test file run or debug it the same way. Run
// with Coverage adds `--coverage` and shows the addon's lines, branches and
// functions as the run covered them (decision T384).
const vscode = require("vscode");
const cp = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const GLOB = "**/*.test.wowlua";
const EXCLUDE = "{**/dist/**,**/node_modules/**,**/.*/**}";

// The package a file belongs to: the nearest folder above it with a
// wowlua.json.
function packageRoot(file) {
    let dir = path.dirname(file);
    for (;;) {
        if (fs.existsSync(path.join(dir, "wowlua.json"))) {
            return dir;
        }
        const up = path.dirname(dir);
        if (up === dir) {
            return undefined;
        }
        dir = up;
    }
}

function lspRange(r) {
    return new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character);
}

// Which of two outcomes a test shows when its flavors differ.
const RANK = { failed: 3, errored: 2, passed: 1, skipped: 0 };

class TestExplorer {
    constructor(context, lsp) {
        this.lsp = lsp;
        this.ctrl = vscode.tests.createTestController("wowlua", "wowlua");
        this.pending = new Map();
        this.ctrl.resolveHandler = async (item) => {
            if (item) {
                await this.load(item);
            } else {
                await this.discover();
            }
        };
        this.ctrl.refreshHandler = () => this.discover();
        this.ctrl.createRunProfile("Run", vscode.TestRunProfileKind.Run, (req, token) => this.run(req, token), true);
        this.ctrl.createRunProfile("Debug", vscode.TestRunProfileKind.Debug, (req) => this.debug(req), true);
        // each file's lines, branches and functions, read when it is shown
        this.covered = new Map();
        const coverage = this.ctrl.createRunProfile(
            "Run with Coverage",
            vscode.TestRunProfileKind.Coverage,
            (req, token) => this.run(req, token, { coverage: true }),
            true,
        );
        coverage.loadDetailedCoverage = async (_run, file) => this.covered.get(file.uri.toString()) || [];
        const watcher = vscode.workspace.createFileSystemWatcher(GLOB);
        // (the code lenses follow the tests as they are read)
        this.lensesChanged = new vscode.EventEmitter();
        context.subscriptions.push(
            this.lensesChanged,
            vscode.languages.registerCodeLensProvider(
                { language: "wowlua", scheme: "file", pattern: "**/*.test.wowlua" },
                {
                    onDidChangeCodeLenses: this.lensesChanged.event,
                    provideCodeLenses: (doc) => this.lenses(doc),
                },
            ),
            vscode.commands.registerCommand("wowlua.runTest", (id) => this.runItem(id, false)),
            vscode.commands.registerCommand("wowlua.debugTest", (id) => this.runItem(id, true)),
            vscode.workspace.onDidChangeConfiguration((e) => {
                if (e.affectsConfiguration("wowlua.testCodeLens")) {
                    this.lensesChanged.fire();
                }
            }),
            this.ctrl,
            watcher,
            watcher.onDidCreate((uri) => this.load(this.file(uri))),
            watcher.onDidChange((uri) => this.later(uri)),
            watcher.onDidDelete((uri) => this.ctrl.items.delete(uri.toString())),
            // (unsaved edits too: the server reads the open text)
            vscode.workspace.onDidChangeTextDocument((e) => {
                if (e.document.uri.path.endsWith(".test.wowlua")) {
                    this.later(e.document.uri);
                }
            }),
        );
    }

    // Every test file of the workspace, with its tests.
    async discover() {
        const files = await vscode.workspace.findFiles(GLOB, EXCLUDE);
        await Promise.all(files.map((uri) => this.load(this.file(uri))));
    }

    // The item of a test file, made the first time.
    file(uri) {
        const id = uri.toString();
        let item = this.ctrl.items.get(id);
        if (!item) {
            const root = packageRoot(uri.fsPath);
            const label = root ? path.relative(root, uri.fsPath).split(path.sep).join("/") : path.basename(uri.fsPath);
            item = this.ctrl.createTestItem(id, label, uri);
            item.canResolveChildren = true;
            this.ctrl.items.add(item);
        }
        return item;
    }

    // A file's tests again, a moment after its last edit.
    later(uri) {
        clearTimeout(this.pending.get(uri.toString()));
        this.pending.set(
            uri.toString(),
            setTimeout(() => this.load(this.file(uri)), 500),
        );
    }

    // A file's tests, as the language server reads them.
    async load(item) {
        if (!item) {
            return;
        }
        let found;
        try {
            found = await this.lsp.request("wowlua/tests", { textDocument: { uri: item.id } });
        } catch {
            found = undefined;
        }
        if (!Array.isArray(found)) {
            return;
        }
        const children = found.map((t) => {
            const child = this.ctrl.createTestItem(`${item.id}::${t.name}`, t.name, item.uri);
            child.range = lspRange(t.range);
            if (t.cases) {
                // (its cases come with a run)
                child.canResolveChildren = false;
            }
            return child;
        });
        item.children.replace(children);
        this.lensesChanged.fire();
    }

    // The code lenses of a test file: run and debug at its top and above
    // each test (`wowlua.testCodeLens` turns them off).
    async lenses(doc) {
        if (!vscode.workspace.getConfiguration("wowlua").get("testCodeLens", true)) {
            return [];
        }
        const file = this.file(doc.uri);
        if (file.children.size === 0) {
            await this.load(file);
        }
        const at = (range, item, label) => [
            new vscode.CodeLens(range, { title: `$(play) ${label}`, command: "wowlua.runTest", arguments: [item.id] }),
            new vscode.CodeLens(range, { title: "$(debug-alt) Debug", command: "wowlua.debugTest", arguments: [item.id] }),
        ];
        const lenses = at(new vscode.Range(0, 0, 0, 0), file, "Run all tests");
        file.children.forEach((t) => {
            if (t.range) {
                lenses.push(...at(new vscode.Range(t.range.start, t.range.start), t, "Run test"));
            }
        });
        return lenses;
    }

    // Runs (or debugs) the test or test file `id` names, as the Testing
    // view would: its results show there and at the test.
    async runItem(id, debug) {
        const item = this.find(id);
        if (!item) {
            return;
        }
        const request = new vscode.TestRunRequest([item]);
        if (debug) {
            await this.debug(request);
        } else {
            await this.run(request, new vscode.CancellationTokenSource().token);
        }
    }

    // The item `id` names: a file's, or one of its tests (or cases).
    find(id) {
        let found;
        const look = (items) =>
            items.forEach((i) => {
                if (found) return;
                if (i.id === id) found = i;
                else look(i.children);
            });
        look(this.ctrl.items);
        return found;
    }

    // The test files and tests a request names, by package: each with the
    // filter that selects it (`tests/a.test.wowlua`, `…::name`).
    plan(request) {
        const chosen = [];
        if (request.include) {
            chosen.push(...request.include);
        } else {
            this.ctrl.items.forEach((item) => chosen.push(item));
        }
        const excluded = new Set((request.exclude || []).map((i) => i.id));
        const packages = new Map();
        for (const item of chosen) {
            if (excluded.has(item.id) || !item.uri) {
                continue;
            }
            const root = packageRoot(item.uri.fsPath);
            if (!root) {
                continue;
            }
            const rel = path.relative(root, item.uri.fsPath).split(path.sep).join("/");
            const filter = isFileItem(item) ? rel : `${rel}::${testName(item)}`;
            if (!packages.has(root)) {
                packages.set(root, []);
            }
            packages.get(root).push({ item, filter });
        }
        return packages;
    }

    command() {
        return vscode.workspace.getConfiguration("wowlua").get("path") || "wowlua";
    }

    async run(request, token, opts = {}) {
        const run = this.ctrl.createTestRun(request);
        const packages = this.plan(request);
        // every test that runs, started
        for (const entries of packages.values()) {
            for (const { item } of entries) {
                const leaves = item.children.size ? [...childrenOf(item)] : [item];
                for (const t of leaves) {
                    run.started(t);
                }
            }
        }
        for (const [root, entries] of packages) {
            if (token.isCancellationRequested) {
                break;
            }
            const out = fs.mkdtempSync(path.join(os.tmpdir(), "wowlua-tests-"));
            const reporters = opts.coverage ? "json,coverage-json" : "json";
            const args = [
                "test",
                ...(opts.coverage ? ["--coverage"] : []),
                "--reporter",
                reporters,
                "--output",
                out,
                "--color",
                "never",
                ...entries.map((e) => e.filter),
            ];
            run.appendOutput(`$ wowlua ${args.map((a) => (a.includes(" ") ? JSON.stringify(a) : a)).join(" ")}\r\n`);
            const code = await new Promise((resolve) => {
                const child = cp.spawn(this.command(), args, { cwd: root });
                const show = (chunk) => run.appendOutput(chunk.toString().replace(/\r?\n/g, "\r\n"));
                child.stdout.on("data", show);
                child.stderr.on("data", show);
                child.on("error", (e) => {
                    run.appendOutput(`cannot start ${this.command()}: ${e.message}\r\n`);
                    resolve(-1);
                });
                child.on("close", resolve);
                token.onCancellationRequested(() => child.kill());
            });
            let report;
            let coverage;
            try {
                report = JSON.parse(fs.readFileSync(path.join(out, "results.json"), "utf8"));
            } catch {
                report = undefined;
            }
            if (opts.coverage) {
                try {
                    coverage = JSON.parse(fs.readFileSync(path.join(out, "coverage.json"), "utf8"));
                } catch {
                    coverage = undefined;
                }
            }
            fs.rmSync(out, { recursive: true, force: true });
            if (coverage) {
                this.addCoverage(run, root, coverage);
            }
            if (!report) {
                const message = new vscode.TestMessage(
                    token.isCancellationRequested ? "cancelled" : `wowlua test gave no results (exit ${code}); see the output`,
                );
                for (const { item } of entries) {
                    for (const t of item.children.size ? [...childrenOf(item)] : [item]) {
                        run.errored(t, message);
                    }
                }
                continue;
            }
            this.report(run, root, report, entries);
        }
        run.end();
    }

    // The results of one package's run, onto its items.
    report(run, root, report, entries) {
        const outcomes = new Map();
        const note = (item, state, message, seconds) => {
            const o = outcomes.get(item.id) || { item, state: "skipped", messages: [], ms: 0, seen: false };
            if (!o.seen || RANK[state] > RANK[o.state]) {
                o.state = state;
            }
            o.seen = true;
            if (message) {
                o.messages.push(message);
            }
            o.ms += (seconds || 0) * 1000;
            outcomes.set(item.id, o);
        };
        const flavors = new Set(report.files.map((f) => f.flavor));
        const prefix = (flavor) => (flavors.size > 1 ? `[${flavor}] ` : "");
        for (const f of report.files) {
            const uri = vscode.Uri.file(path.join(root, f.file));
            const fileItem = this.file(uri);
            if (f.error) {
                // the file did not load: each of its tests requested errs
                const message = new vscode.TestMessage(prefix(f.flavor) + f.error);
                message.location = new vscode.Location(uri, new vscode.Position(0, 0));
                const requested = entries.filter((e) => e.item.uri && e.item.uri.toString() === uri.toString());
                for (const { item } of requested) {
                    for (const t of item.children.size ? [...childrenOf(item)] : [item]) {
                        note(t, "errored", message, 0);
                    }
                }
                continue;
            }
            for (const t of f.tests) {
                const item = this.testItem(fileItem, t.name);
                let message;
                if (t.status === "failed") {
                    message = new vscode.TestMessage(prefix(f.flavor) + (t.message || "failed"));
                    const at = /^(.*):(\d+)$/.exec(t.location || "");
                    if (at) {
                        message.location = new vscode.Location(
                            vscode.Uri.file(path.join(root, at[1])),
                            new vscode.Position(Number(at[2]) - 1, 0),
                        );
                    } else if (item.range) {
                        message.location = new vscode.Location(uri, item.range);
                    }
                }
                note(item, t.status, message, t.seconds);
            }
        }
        // (for the extension's tests: what the last run gave)
        this.last = [...outcomes.values()].map((o) => ({ name: testName(o.item), state: o.state }));
        for (const o of outcomes.values()) {
            switch (o.state) {
                case "passed":
                    run.passed(o.item, o.ms);
                    break;
                case "failed":
                    run.failed(o.item, o.messages, o.ms);
                    break;
                case "errored":
                    run.errored(o.item, o.messages, o.ms);
                    break;
                default:
                    run.skipped(o.item);
            }
        }
    }

    // A coverage report onto the run: each file's totals now, its lines,
    // branches and functions when the editor shows them.
    addCoverage(run, root, coverage) {
        const count = (k) => new vscode.TestCoverageCount(k.covered, k.total);
        this.lastCoverage = [];
        for (const [file, c] of Object.entries(coverage.files || {})) {
            const uri = vscode.Uri.file(path.join(root, file));
            const t = c.totals;
            run.addCoverage(
                new vscode.FileCoverage(
                    uri,
                    count(t.lines),
                    t.branches.total ? count(t.branches) : undefined,
                    t.functions.total ? count(t.functions) : undefined,
                ),
            );
            // (a branch is taken or not: each side its own count)
            const branches = new Map();
            for (const b of c.branches || []) {
                const list = branches.get(b.line) || [];
                list.push(
                    new vscode.BranchCoverage(b.taken, undefined, `branch ${b.index + 1} taken`),
                    new vscode.BranchCoverage(b.notTaken, undefined, `branch ${b.index + 1} not taken`),
                );
                branches.set(b.line, list);
            }
            const details = Object.entries(c.lines || {}).map(([line, hits]) => {
                const l = Number(line) - 1;
                return new vscode.StatementCoverage(hits, new vscode.Range(l, 0, l, Number.MAX_SAFE_INTEGER), branches.get(Number(line)) || []);
            });
            for (const f of c.functions || []) {
                details.push(new vscode.DeclarationCoverage(f.name, f.calls, new vscode.Position(f.line - 1, 0)));
            }
            this.covered.set(uri.toString(), details);
            this.lastCoverage.push({ file, lines: t.lines.covered, of: t.lines.total });
        }
    }

    // The item of the test `name` of a file: its own, a case under the
    // `@cases` test it is one of (`adds [1, 2]` of `adds`), else made.
    testItem(fileItem, name) {
        const id = `${fileItem.id}::${name}`;
        const own = fileItem.children.get(id);
        if (own) {
            return own;
        }
        let parent = fileItem;
        fileItem.children.forEach((c) => {
            if (name.startsWith(`${c.label} `)) {
                parent = c;
            }
        });
        const existing = parent.children.get(id);
        if (existing) {
            return existing;
        }
        const made = this.ctrl.createTestItem(id, parent === fileItem ? name : name.slice(parent.label.length + 1), fileItem.uri);
        if (parent !== fileItem && parent.range) {
            made.range = parent.range;
        }
        parent.children.add(made);
        return made;
    }

    // Debug: the first test or file asked for, in `wowlua debug`.
    async debug(request) {
        const item = (request.include || [])[0];
        if (!item || !item.uri) {
            vscode.window.showInformationMessage("Pick a test or test file to debug.");
            return;
        }
        const root = packageRoot(item.uri.fsPath);
        const rel = root ? path.relative(root, item.uri.fsPath).split(path.sep).join("/") : undefined;
        const folder = vscode.workspace.getWorkspaceFolder(item.uri);
        const config = {
            type: "wowlua",
            request: "launch",
            name: `wowlua: debug ${item.label}`,
            test: item.uri.fsPath,
        };
        if (!isFileItem(item) && rel) {
            config.filter = `${rel}::${testName(item)}`;
        }
        await vscode.debug.startDebugging(folder, config);
    }
}

// Whether an item is a test file's (its id is the file's URI).
function isFileItem(item) {
    return item.uri && item.id === item.uri.toString();
}

// A test's full name (a case's too): its id after the file's URI and `::`.
function testName(item) {
    return item.id.slice(item.uri.toString().length + 2);
}

// A test item's tests, its cases included.
function* childrenOf(item) {
    const list = [];
    item.children.forEach((c) => list.push(c));
    for (const c of list) {
        if (c.children.size) {
            yield* childrenOf(c);
        } else {
            yield c;
        }
    }
}

module.exports = { TestExplorer, packageRoot };
