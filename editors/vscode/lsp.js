// The language client: starts `wowlua lsp` and keeps it in step with the
// open `.wowlua` and `.lua` files (whole texts, unsaved edits included). Its
// diagnostics go to the Problems view; it formats documents, outlines them,
// finds where their names are defined, shows their types on hover,
// completes names and members, and shows a call's parameters. Hand-written on `connection.js`, so the
// extension has no build step (decision T331).
const vscode = require("vscode");
const { Connection } = require("./connection");

// The documents it formats and outlines.
const SELECTOR = [{ language: "wowlua", scheme: "file" }];
// The documents it checks, and finds definitions in.
const SYNCED = [...SELECTOR, { language: "lua", scheme: "file" }];

// The documents the server is told about.
function synced(doc) {
    return doc.uri.scheme === "file" && (doc.languageId === "wowlua" || doc.languageId === "lua");
}

function lspRange(r) {
    return new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character);
}

function diagnostic(d) {
    const out = new vscode.Diagnostic(lspRange(d.range), d.message, (d.severity || 1) - 1);
    out.source = d.source;
    if (d.code !== undefined) {
        out.code = d.code;
    }
    return out;
}

function symbol(s) {
    const out = new vscode.DocumentSymbol(s.name, s.detail || "", s.kind - 1, lspRange(s.range), lspRange(s.selectionRange));
    out.children = (s.children || []).map(symbol);
    return out;
}

class LanguageClient {
    constructor(context) {
        this.context = context;
        this.output = vscode.window.createOutputChannel("wowlua Language Server");
        this.diagnostics = vscode.languages.createDiagnosticCollection("wowlua");
        this.connection = undefined;
        this.ready = undefined;
        // when the server stopped by itself lately (restarted at once up to
        // three times in three minutes)
        this.crashes = [];
        context.subscriptions.push(
            this.output,
            this.diagnostics,
            vscode.workspace.onDidOpenTextDocument((d) => this.opened(d)),
            vscode.workspace.onDidChangeTextDocument((e) => this.changed(e)),
            vscode.workspace.onDidSaveTextDocument((d) => this.saved(d)),
            vscode.workspace.onDidCloseTextDocument((d) => this.closed(d)),
            vscode.languages.registerDocumentFormattingEditProvider(SELECTOR, {
                provideDocumentFormattingEdits: (doc, options) => this.format(doc, options),
            }),
            vscode.languages.registerDocumentSymbolProvider(SELECTOR, {
                provideDocumentSymbols: (doc) => this.symbols(doc),
            }),
            vscode.languages.registerDefinitionProvider(SYNCED, {
                provideDefinition: (doc, position) => this.definition(doc, position),
            }),
            vscode.languages.registerHoverProvider(SYNCED, {
                provideHover: (doc, position) => this.hover(doc, position),
            }),
            vscode.languages.registerCompletionItemProvider(
                SYNCED,
                {
                    provideCompletionItems: (doc, position) => this.complete(doc, position),
                    resolveCompletionItem: (item) => this.resolveCompletion(item),
                },
                ".",
                ":",
                // (in a string: the event names, anchor points, … its type names)
                '"',
                "'",
                // (the decorators that apply to what follows)
                "@",
            ),
            vscode.languages.registerSignatureHelpProvider(
                SYNCED,
                { provideSignatureHelp: (doc, position) => this.signatureHelp(doc, position) },
                { triggerCharacters: ["(", ","], retriggerCharacters: [","] },
            ),
            // files changed on disk (a branch checked out, a formatter
            // run): the server checks their packages again
            ...this.watch("**/*.{wowlua,lua,xml,toc}"),
            ...this.watch("**/wowlua.{json,lock}"),
            vscode.workspace.onDidChangeConfiguration((e) => {
                if (e.affectsConfiguration("wowlua.path") || e.affectsConfiguration("wowlua.languageServer")) {
                    this.restart();
                }
            }),
            { dispose: () => this.stop() },
        );
    }

    // A file system watcher whose changes go to the server.
    watch(pattern) {
        const watcher = vscode.workspace.createFileSystemWatcher(pattern);
        const report = (type) => (uri) => {
            if (uri.scheme === "file") {
                this.send("workspace/didChangeWatchedFiles", { changes: [{ uri: uri.toString(), type }] });
            }
        };
        return [watcher, watcher.onDidCreate(report(1)), watcher.onDidChange(report(2)), watcher.onDidDelete(report(3))];
    }

    enabled() {
        return vscode.workspace.getConfiguration("wowlua").get("languageServer", true);
    }

    // Starts the server and tells it about the open documents.
    start() {
        if (this.connection || !this.enabled()) {
            return;
        }
        const command = vscode.workspace.getConfiguration("wowlua").get("path") || "wowlua";
        const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
        const connection = new Connection(command, folder ? folder.uri.fsPath : undefined);
        this.connection = connection;
        connection.onNotification("textDocument/publishDiagnostics", (p) => {
            this.diagnostics.set(vscode.Uri.parse(p.uri), p.diagnostics.map(diagnostic));
        });
        connection.child.on("error", (e) => {
            this.output.appendLine(`cannot start ${command} lsp: ${e.message}`);
        });
        connection.child.stderr.on("data", (chunk) => this.output.append(chunk.toString()));
        connection.onNotification("window/logMessage", (p) => this.output.appendLine(p.message));
        connection.exited.then((code) => {
            if (this.connection !== connection) {
                return;
            }
            this.connection = undefined;
            this.diagnostics.clear();
            // stopped by itself: started again at once, unless it keeps
            // stopping
            const now = Date.now();
            this.crashes = this.crashes.filter((t) => now - t < 180000).concat(now);
            if (this.crashes.length <= 3) {
                this.output.appendLine(`the server stopped (${code}); starting it again`);
                setTimeout(() => this.start(), 1000);
                return;
            }
            vscode.window
                .showWarningMessage(`The wowlua language server keeps stopping (${code}). Is wowlua 0.10 or newer installed?`, "Restart")
                .then((pick) => {
                    if (pick) {
                        this.crashes = [];
                        this.restart();
                    }
                });
        });
        this.ready = connection
            .request("initialize", {
                processId: process.pid,
                clientInfo: { name: "vscode" },
                rootUri: folder ? folder.uri.toString() : null,
                // (the server reports files in these folders as they are spelled here)
                workspaceFolders: (vscode.workspace.workspaceFolders || []).map((f) => ({ uri: f.uri.toString(), name: f.name })),
                capabilities: {},
            })
            .then(() => {
                connection.notify("initialized", {});
                for (const doc of vscode.workspace.textDocuments) {
                    this.opened(doc);
                }
            });
        this.ready.catch((e) => this.output.appendLine(`initialize failed: ${e.message}`));
    }

    async stop() {
        const connection = this.connection;
        this.connection = undefined;
        this.diagnostics.clear();
        if (connection) {
            await connection.stop();
        }
    }

    async restart() {
        await this.stop();
        this.start();
    }

    // A notification once the server is initialized.
    send(method, params) {
        const connection = this.connection;
        if (connection) {
            this.ready.then(() => connection.notify(method, params), () => {});
        }
    }

    opened(doc) {
        if (synced(doc)) {
            this.send("textDocument/didOpen", {
                textDocument: { uri: doc.uri.toString(), languageId: doc.languageId, version: doc.version, text: doc.getText() },
            });
        }
    }

    // Only what changed: a keystroke in a large file sends a character.
    changed(e) {
        if (synced(e.document) && e.contentChanges.length > 0) {
            this.send("textDocument/didChange", {
                textDocument: { uri: e.document.uri.toString(), version: e.document.version },
                contentChanges: e.contentChanges.map((c) => ({
                    range: {
                        start: { line: c.range.start.line, character: c.range.start.character },
                        end: { line: c.range.end.line, character: c.range.end.character },
                    },
                    text: c.text,
                })),
            });
        }
    }

    // wowlua.json too: a changed manifest checks its package again.
    saved(doc) {
        if (synced(doc) || doc.uri.path.endsWith("/wowlua.json")) {
            this.send("textDocument/didSave", { textDocument: { uri: doc.uri.toString() } });
        }
    }

    closed(doc) {
        if (synced(doc)) {
            this.send("textDocument/didClose", { textDocument: { uri: doc.uri.toString() } });
        }
    }

    async request(method, params) {
        const connection = this.connection;
        if (!connection) {
            return undefined;
        }
        await this.ready;
        return connection.request(method, params);
    }

    async format(doc, options) {
        const edits = await this.request("textDocument/formatting", {
            textDocument: { uri: doc.uri.toString() },
            options: { tabSize: options.tabSize, insertSpaces: options.insertSpaces },
        });
        return (edits || []).map((e) => vscode.TextEdit.replace(lspRange(e.range), e.newText));
    }

    async definition(doc, position) {
        const found = await this.request("textDocument/definition", {
            textDocument: { uri: doc.uri.toString() },
            position: { line: position.line, character: position.character },
        });
        return (found || []).map((l) => new vscode.Location(vscode.Uri.parse(l.uri), lspRange(l.range)));
    }

    // The type of the name under the pointer, and its doc comment.
    async hover(doc, position) {
        const found = await this.request("textDocument/hover", {
            textDocument: { uri: doc.uri.toString() },
            position: { line: position.line, character: position.character },
        });
        if (!found || !found.contents) {
            return undefined;
        }
        return new vscode.Hover(new vscode.MarkdownString(found.contents.value), found.range ? lspRange(found.range) : undefined);
    }

    // What can stand at the cursor: names in scope, a value's members after
    // `.` and `:`, type names in annotations, the strings a string's type
    // names.
    async complete(doc, position) {
        const found = await this.request("textDocument/completion", {
            textDocument: { uri: doc.uri.toString() },
            position: { line: position.line, character: position.character },
        });
        if (!found) {
            return undefined;
        }
        const items = (found.items || []).map((i) => {
            // (the protocol's kinds count from 1, VS Code's from 0)
            const item = new vscode.CompletionItem(i.label, (i.kind || 1) - 1);
            item.detail = i.detail;
            item.sortText = i.sortText;
            item.lsp = i;
            return item;
        });
        return new vscode.CompletionList(items, !!found.isIncomplete);
    }

    // A completion's type (the game's names) and doc comment, read once it
    // is shown.
    async resolveCompletion(item) {
        if (item.lsp && item.lsp.data !== undefined) {
            const found = await this.request("completionItem/resolve", item.lsp);
            if (found && found.detail) {
                item.detail = found.detail;
            }
            if (found && found.documentation) {
                item.documentation = new vscode.MarkdownString(found.documentation.value);
            }
        }
        return item;
    }

    // The parameters of the call the cursor is in, the one it is at
    // highlighted.
    async signatureHelp(doc, position) {
        const found = await this.request("textDocument/signatureHelp", {
            textDocument: { uri: doc.uri.toString() },
            position: { line: position.line, character: position.character },
        });
        if (!found || !found.signatures) {
            return undefined;
        }
        const markdown = (d) => (d ? new vscode.MarkdownString(d.value) : undefined);
        const help = new vscode.SignatureHelp();
        help.signatures = found.signatures.map((s) => {
            const sig = new vscode.SignatureInformation(s.label, markdown(s.documentation));
            sig.parameters = (s.parameters || []).map((p) => new vscode.ParameterInformation(p.label, markdown(p.documentation)));
            return sig;
        });
        help.activeSignature = found.activeSignature || 0;
        help.activeParameter = found.activeParameter || 0;
        return help;
    }

    async symbols(doc) {
        const found = await this.request("textDocument/documentSymbol", { textDocument: { uri: doc.uri.toString() } });
        return (found || []).map(symbol);
    }
}

module.exports = { LanguageClient };
