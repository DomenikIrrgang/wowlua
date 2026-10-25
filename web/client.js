// The page's side of @wowlua/web's worker (worker.js): the playground's
// calls as promises, and the language server as a message transport an
// editor's LSP client can use.
//
//   import { connect } from "@wowlua/web/client";
//   const wowlua = connect(new Worker(new URL("@wowlua/web/worker", import.meta.url), { type: "module" }));
//   await wowlua.call("writeFile", "src/main.wowlua", text);
//   const diagnostics = await wowlua.call("check");
//   wowlua.lsp.onMessage((m) => …);  wowlua.lsp.send({ jsonrpc: "2.0", … });

export function connect(worker, { wasm } = {}) {
    let next = 1;
    const waiting = new Map();
    const listeners = new Set();
    worker.addEventListener("message", (event) => {
        const m = event.data;
        if (m.lsp !== undefined) {
            for (const listen of listeners) listen(m.lsp);
            return;
        }
        const pending = waiting.get(m.id);
        if (!pending) return;
        waiting.delete(m.id);
        if (m.error !== undefined) pending.reject(new Error(m.error));
        else pending.resolve(m.result);
    });
    if (wasm !== undefined) worker.postMessage({ wasm: String(wasm) });
    return {
        /** Calls a Playground method in the worker. */
        call(method, ...args) {
            const id = next++;
            return new Promise((resolve, reject) => {
                waiting.set(id, { resolve, reject });
                worker.postMessage({ id, call: method, args });
            });
        },
        lsp: {
            send(message) {
                worker.postMessage({ lsp: message });
            },
            /** Returns a function that stops listening. */
            onMessage(listen) {
                listeners.add(listen);
                return () => listeners.delete(listen);
            },
        },
        terminate() {
            worker.terminate();
        },
    };
}
