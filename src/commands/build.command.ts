import { Command } from "cli-program-lib/decorators/command.decorator";
import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { Logger } from "cli-program-lib/logging/logger";
import { WowluaConfig } from "../config/wowlua.config";
import * as chokidar from "chokidar"
import { JsonParser } from "cli-program-lib/properties/json.parser"
import { BuildSystem } from "../build-system/build-system";


@Command({
    name: "build",
    description: "Build the project."
})
export class BuildCommand {

    @Inject(WowluaConfig)
    private config: WowluaConfig

    @Inject(Logger)
    private logger: Logger

    @Inject(BuildSystem)
    private buildSystem: BuildSystem

    public async execute(args: String[]): Promise<void> {
        const watch: boolean = args.includes("--watch")
        this.buildSystem.init()
        this.buildSystem.build()
        if (watch) {
            this.logger.info("Watching for file changes in path: " + this.config.getSrcPath())
            let rebuild = () => {
                console.clear()
                this.logger.info("File change detected re-building project...")
                this.buildSystem.build()
                this.logger.info("Watching for file changes in path: " + this.config.getSrcPath())
            }
            chokidar.watch(this.config.getSrcPath(), { ignoreInitial: true }).on("change", rebuild).on("add", rebuild).on("unlink", rebuild).on("unlinkDir", rebuild)
        }
    }

}