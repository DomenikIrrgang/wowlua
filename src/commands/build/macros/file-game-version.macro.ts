import { GameVersion } from "../../../util/game-version";
import { Macro } from "../macro";
import { SourceFile } from "../source-file";

export class FileGameVersionMacro implements Macro {

    public parse(sourceFiles: SourceFile[], sourceFile: SourceFile, args: string[], lineNumberStart: number, lineNumberEnd: number): string[] {
        if (args.length !== 1) {
            throw new Error("Invalid number of arguments for @fileGameVersion: " + args.length + " (expected 1)")
        }
        sourceFile.gameVersionSpecific = true
        sourceFile.gameVersions = this.getIncludedGameVersions(args)
        return []
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

}