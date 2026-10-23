// The wowlua debug type: VS Code starts `wowlua debug` in the workspace
// folder and talks the Debug Adapter Protocol with it over stdio.
const vscode = require("vscode");

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
    context.subscriptions.push(
        vscode.debug.registerDebugAdapterDescriptorFactory("wowlua", factory),
    );
}

function deactivate() {}

module.exports = { activate, deactivate };
