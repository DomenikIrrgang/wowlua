import { Command } from "cli-program-lib/decorators/command.decorator"
import { WowluaConfig } from "../config/wowlua.config"
import { Inject } from "cli-program-lib/decorators/inject.decorator"
import { Logger } from "cli-program-lib/logging/logger"
import { DependencyManager } from "../dependency-manager/dependency-manager"

@Command({
    name: "dependency-remove",
    description: "Adds a new dependency to the project and installs it."
})
export class DependencyRemove {

    @Inject(WowluaConfig)
    private config: WowluaConfig

    @Inject(Logger)
    private logger: Logger

    @Inject(DependencyManager)
    private dependencyManager: DependencyManager

    public execute(args: String[]): void {
        if (args[3] === undefined) {
            this.logger.error("Dependency name not specified. Correct usage: wowlua dependency-remove <name>")
            return
        }
        let dependency = this.config.dependencies.find((dependency) => dependency.name === args[3])
        if (dependency === undefined) {
            this.logger.error("Dependency " + args[3] + " not found")
            return
        }
        this.dependencyManager.removeDependency(dependency)
        this.dependencyManager.uninstallDependency(dependency)
    }

}