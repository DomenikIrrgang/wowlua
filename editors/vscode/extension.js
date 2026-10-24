// The wowlua debug type: VS Code starts `wowlua debug` in the workspace
// folder and talks the Debug Adapter Protocol with it over stdio.
const vscode = require("vscode");

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

function activate(context) {
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
    context.subscriptions.push(
        vscode.debug.registerDebugAdapterDescriptorFactory("wowlua", factory),
        vscode.debug.registerDebugConfigurationProvider("wowlua", provider),
    );
}

function deactivate() {}

module.exports = { activate, deactivate };
