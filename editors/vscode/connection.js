// A JSON-RPC connection to `wowlua lsp` over its stdio, framed as the
// Language Server Protocol frames it (`Content-Length: N`, a blank line, N
// bytes of JSON). Plain Node: the extension's language client builds on it,
// and its tests use it without VS Code.
const { spawn } = require("child_process");

class Connection {
    // Starts `command lsp` in `cwd`.
    constructor(command, cwd) {
        this.child = spawn(command, ["lsp"], { cwd, stdio: ["pipe", "pipe", "pipe"] });
        this.nextId = 1;
        this.pending = new Map();
        this.handlers = new Map();
        this.buffer = Buffer.alloc(0);
        this.stderr = "";
        this.exited = new Promise((resolve) => {
            this.child.on("exit", (code) => {
                for (const { reject } of this.pending.values()) {
                    reject(new Error(`wowlua lsp exited (${code})`));
                }
                this.pending.clear();
                resolve(code);
            });
        });
        this.child.on("error", (e) => {
            for (const { reject } of this.pending.values()) reject(e);
            this.pending.clear();
        });
        this.child.stdout.on("data", (chunk) => this.receive(chunk));
        // (the last of what it wrote, for an error message)
        this.child.stderr.on("data", (chunk) => {
            this.stderr = (this.stderr + chunk.toString()).slice(-65536);
        });
    }

    // Calls `handler(params)` for each notification `method` from the server.
    onNotification(method, handler) {
        this.handlers.set(method, handler);
    }

    // A request; resolves with its result, rejects with its error.
    request(method, params) {
        const id = this.nextId++;
        return new Promise((resolve, reject) => {
            this.pending.set(id, { resolve, reject });
            this.send({ jsonrpc: "2.0", id, method, params });
        });
    }

    notify(method, params) {
        this.send({ jsonrpc: "2.0", method, params });
    }

    send(message) {
        if (!this.child.stdin.writable) {
            return;
        }
        const body = Buffer.from(JSON.stringify(message), "utf8");
        this.child.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
        this.child.stdin.write(body);
    }

    receive(chunk) {
        this.buffer = Buffer.concat([this.buffer, chunk]);
        for (;;) {
            const end = this.buffer.indexOf("\r\n\r\n");
            if (end < 0) {
                return;
            }
            const header = this.buffer.subarray(0, end).toString("ascii");
            const match = /content-length:\s*(\d+)/i.exec(header);
            const length = match ? Number(match[1]) : 0;
            if (this.buffer.length < end + 4 + length) {
                return;
            }
            const body = this.buffer.subarray(end + 4, end + 4 + length).toString("utf8");
            this.buffer = this.buffer.subarray(end + 4 + length);
            let message;
            try {
                message = JSON.parse(body);
            } catch {
                continue;
            }
            this.dispatch(message);
        }
    }

    dispatch(message) {
        if (message.id !== undefined && !message.method) {
            const waiting = this.pending.get(message.id);
            if (!waiting) {
                return;
            }
            this.pending.delete(message.id);
            if (message.error) {
                waiting.reject(new Error(message.error.message));
            } else {
                waiting.resolve(message.result);
            }
        } else if (message.method) {
            const handler = this.handlers.get(message.method);
            if (handler) {
                handler(message.params);
            }
        }
    }

    // `shutdown`, then `exit`.
    async stop() {
        try {
            await Promise.race([this.request("shutdown", null), this.exited]);
        } catch {
            // already gone
        }
        this.notify("exit", null);
        this.child.stdin.end();
        await Promise.race([this.exited, new Promise((r) => setTimeout(r, 2000))]);
        this.child.kill();
    }
}

module.exports = { Connection };
