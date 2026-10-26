// Serves web/ for the test page, cross-origin isolated (COOP and COEP), as
// the debugger needs: `node demo/serve.mjs [port]`, then
// http://localhost:8000/demo/.
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const types = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".mjs": "text/javascript; charset=utf-8",
    ".wasm": "application/wasm",
    ".json": "application/json",
};
const port = Number(process.argv[2] ?? 8000);
createServer(async (req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname));
    let file = join(root, path);
    if (!file.startsWith(root)) {
        res.writeHead(403).end();
        return;
    }
    if (path.endsWith("/")) file = join(file, "index.html");
    try {
        const body = await readFile(file);
        res.writeHead(200, {
            "Content-Type": types[extname(file)] ?? "application/octet-stream",
            "Cross-Origin-Opener-Policy": "same-origin",
            "Cross-Origin-Embedder-Policy": "require-corp",
        });
        res.end(body);
    } catch {
        res.writeHead(404).end();
    }
}).listen(port, "127.0.0.1", () => console.log(`http://127.0.0.1:${port}/demo/`));
