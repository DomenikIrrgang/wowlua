// The coverage report in an editor tab: the HTML report `wowlua test
// --coverage` writes (`coverage.html`), shown in a webview. It shows the
// latest report: the Testing view's last run with coverage, or the newest
// `coverage.html` in the workspace; a report named in the Explorer opens
// that one. A line's number in it opens the line in the editor.
const vscode = require("vscode");
const fs = require("fs");
const path = require("path");

const VIEW_TYPE = "wowlua.coverage";

// The package a report belongs to: the nearest folder up with a
// wowlua.json, else the report's own.
function packageOf(file) {
    let dir = path.dirname(file);
    for (;;) {
        if (fs.existsSync(path.join(dir, "wowlua.json"))) {
            return dir;
        }
        const up = path.dirname(dir);
        if (up === dir) {
            return path.dirname(file);
        }
        dir = up;
    }
}

function nonce() {
    let out = "";
    const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    for (let i = 0; i < 32; i++) {
        out += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return out;
}

// The report's page for the webview: its scripts allowed by a nonce, and
// the VS Code API handed to its script.
function page(html) {
    const n = nonce();
    const csp = `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src 'nonce-${n}';">`;
    const bridge = `<script nonce="${n}">window.wowluaVscode = acquireVsCodeApi();</script>`;
    return html
        .replace("<head>", `<head>${csp}`)
        .replace(/<script>/g, `<script nonce="${n}">`)
        .replace('<script type="application/json" id="data">', `${bridge}<script type="application/json" id="data">`);
}

class CoverageReport {
    constructor(context) {
        this.context = context;
        this.panel = undefined;
        // what the panel shows: { root, html, at }
        this.shown = undefined;
        // the Testing view's last report with coverage
        this.latest = undefined;
        // the views the page drew, for the tests
        this.views = [];
        this.waiters = [];
    }

    // The Testing view ran with coverage: its report, shown at once when
    // the panel is open.
    ranWithCoverage(root, html) {
        this.latest = { root, html, at: Date.now() };
        if (this.panel) {
            this.display(this.latest);
        }
    }

    // The command: the report `uri` names (the Explorer's), else the latest.
    async show(uri) {
        let report;
        if (uri instanceof vscode.Uri) {
            report = this.read(uri.fsPath);
        } else {
            report = await this.newest();
        }
        if (!report) {
            const pick = await vscode.window.showInformationMessage(
                "No coverage report yet: run the tests with coverage (the Testing view), or `wowlua test --coverage --reporter html`.",
                "Run All Tests with Coverage",
            );
            if (pick) {
                await vscode.commands.executeCommand("testing.coverageAll");
            }
            return false;
        }
        this.display(report);
        return true;
    }

    read(file) {
        try {
            return { root: packageOf(file), html: fs.readFileSync(file, "utf8"), at: fs.statSync(file).mtimeMs };
        } catch {
            return undefined;
        }
    }

    // The latest report: the Testing view's or the newest on disk.
    async newest() {
        let best = this.latest;
        const found = await vscode.workspace.findFiles("**/test/coverage.html", "**/node_modules/**", 20);
        for (const uri of found) {
            let at;
            try {
                at = fs.statSync(uri.fsPath).mtimeMs;
            } catch {
                continue;
            }
            if (!best || at > best.at) {
                best = this.read(uri.fsPath) || best;
            }
        }
        return best;
    }

    display(report) {
        this.shown = report;
        if (!this.panel) {
            this.panel = vscode.window.createWebviewPanel(
                VIEW_TYPE,
                "Coverage",
                vscode.ViewColumn.Active,
                { enableScripts: true, retainContextWhenHidden: true },
            );
            this.panel.iconPath = vscode.Uri.joinPath(this.context.extensionUri, "icon.png");
            this.panel.webview.onDidReceiveMessage((m) => this.message(m));
            this.panel.onDidDispose(() => {
                this.panel = undefined;
            });
        } else {
            this.panel.reveal();
        }
        const title = /<title>(.*?) coverage<\/title>/.exec(report.html);
        this.panel.title = title ? `Coverage: ${title[1]}` : "Coverage";
        this.panel.webview.html = page(report.html);
    }

    async message(m) {
        if (m.type === "shown") {
            this.views.push(m.here);
            const waiting = this.waiters;
            this.waiters = [];
            waiting.forEach((w) => w(m.here));
        } else if (m.type === "open") {
            await this.open(m.path, m.line);
        }
    }

    // A source of the report, by its path in the package (or in one of the
    // workspace's addons), at a line.
    async open(rel, line) {
        const root = this.shown ? this.shown.root : undefined;
        let uri;
        if (root && fs.existsSync(path.join(root, rel))) {
            uri = vscode.Uri.file(path.join(root, rel));
        } else {
            const base = root ? new vscode.RelativePattern(root, `**/${rel}`) : `**/${rel}`;
            const found = await vscode.workspace.findFiles(base, "**/node_modules/**", 1);
            uri = found[0];
        }
        if (!uri) {
            vscode.window.showWarningMessage(`${rel} is not in the workspace`);
            return undefined;
        }
        const at = new vscode.Position(Math.max(0, line - 1), 0);
        return vscode.window.showTextDocument(uri, {
            viewColumn: vscode.ViewColumn.Beside,
            selection: new vscode.Range(at, at),
            preview: true,
        });
    }

    // For the tests: asks the page to show `hash` or click `selector`, and
    // waits until it drew a view.
    post(m) {
        const drawn = new Promise((resolve) => this.waiters.push(resolve));
        this.panel.webview.postMessage(m);
        return drawn;
    }

    // Waits for the page's next view.
    drawn() {
        return new Promise((resolve) => this.waiters.push(resolve));
    }
}

module.exports = { CoverageReport, page, packageOf };
