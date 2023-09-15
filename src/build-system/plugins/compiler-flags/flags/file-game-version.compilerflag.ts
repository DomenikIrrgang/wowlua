import { GameVersion } from "../../../../util/game-version"
import { BuildContext } from "../../../build-context"
import { SourceFile } from "../../../source-file"
import { CompilerFlag } from "./compilerflag"

export class FileGameVersionCompilerFlag implements CompilerFlag {

    public parse(buildContext: BuildContext, sourceFile: SourceFile, args: string[], lineNumberStart: number, lineNumberEnd: number): string {
        if (args.length < 1) {
            throw new Error("Invalid number of arguments for FileGameVersion: " + args.length + " (expected at least 1)")
        }
        sourceFile.gameVersionSpecific = true
        sourceFile.gameVersions = this.getIncludedGameVersions(args)
        return "-- This file is for " + sourceFile.gameVersions.join(", ") + " only."
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