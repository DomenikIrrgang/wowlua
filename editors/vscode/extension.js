// The wowlua debug type: VS Code starts `wowlua debug` in the workspace
// folder and talks the Debug Adapter Protocol with it over stdio. The UI
// panel shows the game's screen; the Frames view its frames.
const vscode = require("vscode");
const { UiPanel } = require("./ui");
const { FramesView } = require("./frames");

// The flavors the package's wowlua.json names, in order.
async function packageFlavors(folder) {
    if (!folder) {
        return [];
    }
    try {
        const uri = vscode.Uri.joinPath(folder.uri, "wowlua.json");
        const text = new TextDecoder().decode(await vscode.workspace.fs.readFile(uri));
        const flavors = JSON.parse(text).flavors;
        return flavors && typeof flavors === "object" ? Object.keys(flavors) : [];
    } catch {
        return [];
    }
}

const RESUMING = new Set(["continue", "next", "stepIn", "stepOut", "stepBack", "reverseContinue"]);

function activate(context) {
    const ui = new UiPanel(context);
    const frames = new FramesView(ui);

    const factory = {
        createDebugAdapterDescriptor(session) {
            const setting = vscode.workspace.getConfiguration("wowlua").get("path");
            const command = session.configuration.wowluaPath || setting || "wowlua";
            const folder = session.workspaceFolder;
            const options = folder ? { cwd: folder.uri.fsPath } : undefined;
            return new vscode.DebugAdapterExecutable(command, ["debug"], options);
        },
    };
    // A session runs one flavor: without one in the configuration, the
    // package's only flavor, or the one picked from its flavors.
    const provider = {
        async resolveDebugConfiguration(folder, config) {
            if (!config.type && !config.request) {
                config = { type: "wowlua", request: "launch", name: "wowlua: run the addon", addon: true };
            }
            if (config.flavor) {
                return config;
            }
            const flavors = await packageFlavors(folder);
            if (flavors.length === 1) {
                config.flavor = flavors[0];
            } else if (flavors.length > 1) {
                const picked = await vscode.window.showQuickPick(flavors, {
                    placeHolder: "The flavor to debug (set \"flavor\" in launch.json to skip this)",
                });
                if (!picked) {
                    return undefined;
                }
                config.flavor = picked;
            }
            return config;
        },
    };
    // The session's stops and runs, for the panel and the view.
    const tracker = {
        createDebugAdapterTracker(session) {
            return {
                onWillReceiveMessage(m) {
                    if (m.type === "request" && RESUMING.has(m.command) && session === ui.session) {
                        ui.continued();
                    }
                },
                onDidSendMessage(m) {
                    if (m.type !== "event" || session !== ui.session) {
                        return;
                    }
                    if (m.event === "stopped") {
                        ui.stopped();
                        frames.refresh();
                    } else if (m.event === "continued") {
                        ui.continued();
                    }
                },
            };
        },
    };
    context.subscriptions.push(
        vscode.debug.registerDebugAdapterDescriptorFactory("wowlua", factory),
        vscode.debug.registerDebugConfigurationProvider("wowlua", provider),
        vscode.debug.registerDebugAdapterTrackerFactory("wowlua", tracker),
        vscode.window.registerTreeDataProvider("wowlua.frames", frames),
        vscode.debug.onDidStartDebugSession((s) => {
            if (s.type === "wowlua") {
                ui.attach(s);
                frames.attach(s);
            }
        }),
        vscode.debug.onDidTerminateDebugSession((s) => {
            ui.detach(s);
            frames.detach(s);
        }),
        vscode.commands.registerCommand("wowlua.showUI", () => ui.show(false)),
        vscode.commands.registerCommand("wowlua.refreshFrames", () => frames.refresh()),
        vscode.commands.registerCommand("wowlua.outlineFrame", (node) => {
            if (node && node.rect) {
                ui.outline(node.rect);
            }
        }),
    );
    // For the extension's tests.
    return { ui, frames };
}

function deactivate() {}

module.exports = { activate, deactivate };
