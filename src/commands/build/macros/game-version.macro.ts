import { GameVersion } from "../../../util/game-version";
import { Macro } from "../macro";
import { SourceFile } from "../source-file";

export class GameVersionMacro implements Macro {

    public parse(sourceFiles: SourceFile[], sourceFile: SourceFile, args: string[], lineNumberStart: number, lineNumberEnd: number): string[] {
        if (args.length < 1) {
            throw new Error("Invalid number of arguments for @GameVersion: " + args.length + " (expected >=1)")
        }

        if (sourceFile.gameVersionSpecific === false) {
            sourceFile.gameVersionSpecific = true
            sourceFile.gameVersions = this.getExcludedGameVersions(args)
        }
        
        let gameVersions = this.getIncludedGameVersions(args)
        for (let gameVersion of gameVersions) { 
            if (!sourceFile.gameVersions.includes(gameVersion)) {
                if (sourceFiles.find(sourceFile => sourceFile.gameVersions?.includes(gameVersion)) === undefined) {
                    sourceFiles.push({
                        path: sourceFile.path.substring(0, sourceFile.path.lastIndexOf("/")) + sourceFile.path.substring(sourceFile.path.lastIndexOf("/") + 1).split(".")[0] + "_" + gameVersion + ".lua",
                        fileName: sourceFile.fileName.substring(0, sourceFile.fileName.lastIndexOf(".")) + "_" + gameVersion + ".lua",
                        gameVersionSpecific: true,
                        gameVersions: [gameVersion],
                        originalCode: sourceFile.originalCode,
                        parsedCode: sourceFile.parsedCode,
                        usedGlobals: sourceFile.usedGlobals,
                        declaredGlobals: sourceFile.declaredGlobals,
                        importedGlobals: sourceFile.importedGlobals,
                    })
                }
                return []
            } else {
                return sourceFile.originalCode.slice(lineNumberStart + 1, lineNumberEnd)
            }
        }
    }

    private getIncludedGameVersions(args: string[]): GameVersion[] {
        let gameVersions: GameVersion[] = []
        args.forEach(arg => {
            if (Object.values(GameVersion).includes(arg as GameVersion)) {
                gameVersions.push(arg as GameVersion)
            } else {
                throw new Error("Invalid game version: " + arg + "." + " Valid game versions are: " + Object.values(GameVersion).join(", "))
            }
        })
        return gameVersions
    }

    private getExcludedGameVersions(args: string[]): GameVersion[] {
        let includedGameVersions = this.getIncludedGameVersions(args)
        let gameVersions: GameVersion[] = []
        Object.values(GameVersion).forEach(gameVersion => {
            if (!includedGameVersions.includes(gameVersion)) {
                gameVersions.push(gameVersion)
            }
        })
        return gameVersions
    }

}