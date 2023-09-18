import { Command } from "cli-program-lib/decorators/command.decorator";
import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { WowluaConfig } from "../config/wowlua.config";
import { Logger } from "cli-program-lib/logging/logger";
import { DependencyManager } from "../dependency-manager/dependency-manager";

@Command({
    name: "install",
    description: "Install dependencies of the project."
})
export class InstallCommand {

    @Inject(WowluaConfig)
    private config: WowluaConfig;

    @Inject(Logger)
    private logger: Logger;

    @Inject(DependencyManager)
    private dependencyManager: DependencyManager;

    public async execute(args: String[]): Promise<void> {
        this.logger.debug("Installing dependencies");
        if (args[3] !== undefined) {
            const dependency = this.config.dependencies.find((dependency) => dependency.name === args[3])
            if (dependency === undefined) {
                throw new Error("Dependency " + args[3] + " not found")
            }
            await this.dependencyManager.installDependency(dependency)
        } else {
            await this.dependencyManager.installAllDependencies()
        }
    }

}