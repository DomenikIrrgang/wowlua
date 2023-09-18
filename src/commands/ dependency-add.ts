import { Command } from "cli-program-lib/decorators/command.decorator"
import { WowluaConfig } from "../config/wowlua.config"
import { Inject } from "cli-program-lib/decorators/inject.decorator"
import { Logger } from "cli-program-lib/logging/logger"
import { Dependency } from "../dependency-manager/dependency"
import { DependencyType } from "../dependency-manager/dependency-type"
import { DependencyManager } from "../dependency-manager/dependency-manager"

@Command({
    name: "dependency-add",
    description: "Adds a new dependency to the project and installs it."
})
export class DependencyAdd {

    @Inject(WowluaConfig)
    private config: WowluaConfig

    @Inject(Logger)
    private logger: Logger

    @Inject(DependencyManager)
    private dependencyManager: DependencyManager

    public execute(args: String[]): void {
        let dependency: Dependency = {
            name: prompt("Name: ", "Ace-3.0"),
            type: prompt("Source Type (" + Object.values(DependencyType) + "): ", DependencyType.GIT) as DependencyType,
            source: prompt("Source (url or path): ", "https://github.com/WoWUIDev/Ace3"),
            path: prompt("Path: ", ""),
            version: prompt("Dependency version: ", "master"),
        }
        this.dependencyManager.addDependency(dependency)
        this.dependencyManager.installDependency(dependency)
    }

}