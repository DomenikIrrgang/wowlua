import * as fs from "fs"

export function readDirSyncRecursive(directoryPath: string): string[] {
    // -- normalize path separator to '/' instead of path.sep, 
    // -- as / works in node for Windows as well, and mixed \\ and / can appear in the path
    directoryPath = directoryPath.replace(/\\/g,'/');  
    let files = fs.readdirSync(directoryPath, { withFileTypes: true })
    let result = []
    for (let file of files) {
        if (file.isDirectory()) {
            result = result.concat(readDirSyncRecursive(directoryPath + "/" + file.name))
        } else {
            result.push(directoryPath + "/" + file.name)
        }
    }
    return result
  }