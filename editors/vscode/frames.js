// The Frames view: the game's shown frames as a tree (`wowlua/frameTree`),
// in the debug sidebar; selecting one outlines it in the UI panel.
const vscode = require("vscode");

class FramesView {
    constructor(ui) {
        this.ui = ui;
        this.session = undefined;
        this.roots = [];
        this.changed = new vscode.EventEmitter();
        this.onDidChangeTreeData = this.changed.event;
        this.loading = false;
    }

    attach(session) {
        this.session = session;
        this.refresh();
    }

    detach(session) {
        if (this.session === session) {
            this.session = undefined;
        }
    }

    /// Loads the tree again; one asked for while a load runs follows it,
    /// so the view ends with the newest frames. Resolves once loaded.
    refresh() {
        if (!this.session) {
            return Promise.resolve();
        }
        if (this.loading) {
            this.again = true;
            return this.loading;
        }
        this.loading = (async () => {
            do {
                this.again = false;
                try {
                    const tree = await this.session.customRequest("wowlua/frameTree", { hidden: false });
                    this.roots = Array.isArray(tree) ? tree : tree ? [tree] : [];
                } catch (e) {
                    this.roots = [];
                }
                this.changed.fire();
                this.ui.setFrames(this.roots);
            } while (this.again && this.session);
            this.loading = undefined;
        })();
        return this.loading;
    }

    getChildren(node) {
        return node ? node.children || [] : this.roots;
    }

    getTreeItem(node) {
        const label = node.name || `${node.type}#${node.id}`;
        const hasChildren = Array.isArray(node.children) && node.children.length > 0;
        const item = new vscode.TreeItem(
            label,
            hasChildren ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None,
        );
        const r = node.rect;
        const size = r ? `${r.width}×${r.height}` : "no rectangle";
        item.description = `${node.type} ${size}${node.text ? ` "${node.text}"` : ""}`;
        item.tooltip = new vscode.MarkdownString(
            [
                `**${label}** (${node.type})`,
                r ? `at ${r.x}, ${r.y}, ${r.width} × ${r.height} screen pixels` : "no rectangle (not placed)",
                `shown: ${node.shown}, visible: ${node.visible}`,
                node.text ? `text: ${node.text}` : "",
            ]
                .filter(Boolean)
                .join("\n\n"),
        );
        item.iconPath = new vscode.ThemeIcon(node.visible ? "symbol-class" : "eye-closed");
        if (r) {
            item.command = { command: "wowlua.outlineFrame", title: "Outline in the UI", arguments: [node] };
            item.contextValue = "frame";
        }
        return item;
    }

    /// For tests: a frame the view has, by its id.
    find(id) {
        const walk = (nodes) => {
            for (const n of nodes) {
                if (n.id === id) {
                    return n;
                }
                const found = walk(n.children || []);
                if (found) {
                    return found;
                }
            }
            return undefined;
        };
        return walk(this.roots);
    }
}

module.exports = { FramesView };
