export function getPath(path: string): string {
    if (path.startsWith("/")) {
        return path
    }
    if (path[1] === ":") {
        return path
    }
    if (path[0] === "~") {
        return process.env.HOME + "/" + path.substring(1)
    }
    return process.cwd() + "/" + path
}