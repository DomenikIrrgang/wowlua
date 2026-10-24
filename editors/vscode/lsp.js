// The language client: starts `wowlua lsp` and keeps it in step with the
// open `.wowlua` and `.lua` files (whole texts, unsaved edits included). Its
// diagnostics go to the Problems view; it formats documents and outlines
// them. Hand-written on `connection.js`, so the extension has no build
// step (decision T331).
const vscode = require("vscode");
const { Connection } = require("./connection");

// The documents it formats and outlines.
const SELECTOR = [{ language: "wowlua", scheme: "file" }];

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
            vscode.workspace.onDidChangeConfiguration((e) => {
                if (e.affectsConfiguration("wowlua.path") || e.affectsConfiguration("wowlua.languageServer")) {
                    this.restart();
                }
            }),
            { dispose: () => this.stop() },
        );
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
        connection.exited.then((code) => {
            if (connection.stderr) {
                this.output.appendLine(connection.stderr);
            }
            if (this.connection !== connection) {
                return;
            }
            this.connection = undefined;
            this.diagnostics.clear();
            vscode.window
                .showWarningMessage(`The wowlua language server stopped (${code}). Is wowlua 0.10 or newer installed?`, "Restart")
                .then((pick) => pick && this.restart());
        });
        this.ready = connection
            .request("initialize", {
                processId: process.pid,
                clientInfo: { name: "vscode" },
                rootUri: folder ? folder.uri.toString() : null,
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

    changed(e) {
        if (synced(e.document) && e.contentChanges.length > 0) {
            this.send("textDocument/didChange", {
                textDocument: { uri: e.document.uri.toString(), version: e.document.version },
                contentChanges: [{ text: e.document.getText() }],
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

    async symbols(doc) {
        const found = await this.request("textDocument/documentSymbol", { textDocument: { uri: doc.uri.toString() } });
        return (found || []).map(symbol);
    }
}

module.exports = { LanguageClient };
