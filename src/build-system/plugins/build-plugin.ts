import { GameVersion } from "../../util/game-version";
import { BuildContext } from "../build-context";
import { SourceFile } from "../source-file";

export interface BuildPlugin {
    parse(buildContext: BuildContext, sourceFile: SourceFile): void
    build(buildContext: BuildContext, gameVersion: GameVersion): void
}