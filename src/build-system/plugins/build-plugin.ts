import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { GameVersion } from "../../util/game-version";
import { BuildContext } from "../build-context";
import { SourceFile } from "../source-file";
import { PluginOptions } from "./plugin-options";
import { WowluaConfig } from "../../config/wowlua.config";
import { Injectable } from "cli-program-lib/decorators/injectable.decorator";
import { injector } from "cli-program-lib/dependency-injection/injector";

@Injectable()
export abstract class BuildPlugin {
    
    public options: PluginOptions = injector.getInstance(WowluaConfig).getPluginOptions(this.constructor.name)
    
    abstract parse(buildContext: BuildContext, sourceFile: SourceFile): void
    abstract build(buildContext: BuildContext, gameVersion: GameVersion): void
    abstract ast(sourceFile: SourceFile, node): void

}