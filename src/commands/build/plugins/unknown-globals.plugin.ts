import { Injectable } from "cli-program-lib/decorators/injectable.decorator";
import { GameVersion } from "../../../util/game-version";
import { BuildContext } from "../build-context";
import { SourceFile } from "../source-file";
import { BuildPlugin } from "./build-plugin";
import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { Logger } from "cli-program-lib/logging/logger";

@Injectable()
export class UnknownGlobalsPlugin implements BuildPlugin {

    @Inject(Logger)
    private logger: Logger
    
    public parse(buildContext: BuildContext, sourceFile: SourceFile): void {}

    public build(buildContext: BuildContext, gameVersion: GameVersion): void {
        let sourceFiles = buildContext.sourceFiles.concat(buildContext.libFiles)
        for (let sourceFile of sourceFiles) {
            for (let importedGlobal of sourceFile.importedGlobals) {
                let found = false
                for (let otherSourceFile of sourceFiles) {
                    if (otherSourceFile.declaredGlobals.includes(importedGlobal)) {
                        found = true
                        break
                    }
                }
                if (found === false) {
                    this.logger.warning("Using potentially undefined global: " + importedGlobal + " in file: " + sourceFile.path + "/" + sourceFile.fileName + "")
                }
            }
        }
    }
}