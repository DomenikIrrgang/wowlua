// The user's WoW installation in the worker (spec M14 S5, decision T398):
// the files of the picked folder the CASC reader needs, indexed by path,
// read synchronously (`FileReaderSync`, a worker's) when wowlua asks. Never
// uploaded: the bytes go from the user's disk into the worker's memory.
//
// What is indexed: `.build.info`, `Data/config/**`, `Data/data/*` and each
// client folder's (`_classic_beta_/`) `.flavor.info` and `Cache/**`.

const norm = (path) => path.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "").toLowerCase();

/** Whether wowlua reads below `parts` (a path's parts from the folder). */
function wanted(parts, isDir) {
    const [first, second] = parts.map((p) => p.toLowerCase());
    if (parts.length === 1) return isDir ? first === "data" || /^_.*_$/.test(first) : true;
    if (first === "data") return second === "config" || second === "data";
    if (/^_.*_$/.test(first)) return second === "cache" || (parts.length === 2 && second === ".flavor.info");
    return false;
}

export class Installation {
    constructor() {
        /** Lower-case path → File. */
        this.files = new Map();
        /** Lower-case folder → its entries' names. */
        this.dirs = new Map([["", new Set()]]);
    }

    add(path, file) {
        const parts = path.split("/").filter(Boolean);
        for (let i = 0; i < parts.length; i++) {
            const dir = norm(parts.slice(0, i).join("/"));
            if (!this.dirs.has(dir)) this.dirs.set(dir, new Set());
            this.dirs.get(dir).add(parts[i]);
        }
        this.files.set(norm(path), file);
    }

    /** From a folder picked with `showDirectoryPicker()`. */
    static async fromHandle(root) {
        const install = new Installation();
        const walk = async (dir, parts) => {
            for await (const [name, handle] of dir.entries()) {
                const here = [...parts, name];
                const isDir = handle.kind === "directory";
                if (!wanted(here, isDir)) continue;
                if (isDir) await walk(handle, here);
                else install.add(here.join("/"), await handle.getFile());
            }
        };
        await walk(root, []);
        return install;
    }

    /**
     * From the files of a folder `<input type="file" webkitdirectory>`
     * (their paths start with the folder's own name), or `{ path, file }`
     * pairs with paths from the folder (a dropped folder's entries).
     */
    static fromFiles(files) {
        const install = new Installation();
        for (const item of files) {
            const [path, file] = item instanceof Blob
                ? [item.webkitRelativePath.split("/").slice(1).join("/"), item]
                : [item.path, item.file];
            const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
            if (parts.length > 0 && wanted(parts, false)) install.add(parts.join("/"), file);
        }
        return install;
    }

    /** The functions wowlua's page storage calls (`installation.rs`). */
    expose() {
        const reader = new FileReaderSync();
        const read = (file, start, end) => new Uint8Array(reader.readAsArrayBuffer(file.slice(start, end)));
        globalThis.wowluaInstallRead = (path) => {
            const file = this.files.get(norm(path));
            return file ? read(file, 0, file.size) : undefined;
        };
        globalThis.wowluaInstallReadAt = (path, offset, length) => {
            const file = this.files.get(norm(path));
            if (!file || offset + length > file.size) return undefined;
            return read(file, offset, offset + length);
        };
        globalThis.wowluaInstallList = (dir) => JSON.stringify([...(this.dirs.get(norm(dir)) ?? [])]);
    }
}
