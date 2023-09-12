import { Command } from "cli-program-lib/decorators/command.decorator"
import { WowluaConfig } from "../config/wowlua.config"
import { Inject } from "cli-program-lib/decorators/inject.decorator"
import { Logger } from "cli-program-lib/logging/logger"
//import * as readline from "readline-sync"
import { GameVersion } from "../util/game-version"

@Command({
    name: "init",
    description: "Initialize a new wowlua project."
})
export class InitCommand {

    @Inject(WowluaConfig)
    private config: WowluaConfig

    @Inject(Logger)
    private logger: Logger

    public execute(... args: String[]): void {
        this.logger.info("Initializing wowlua project...")
        /*let projectName = readline.question("Project name: ")
        this.config.name = (projectName !== "") ? projectName : this.config.name
        let projectDescription = readline.question("Project description: ")
        this.config.description = (projectDescription !== "") ? projectDescription : this.config.description
        let projectVersion = readline.question("Project version: ")
        this.config.version = (projectVersion !== "") ? projectVersion : this.config.version
        let projectAuthor = readline.question("Project author: ")
        this.config.author = (projectAuthor !== "") ? projectAuthor : this.config.author
        let projectGameVersions = readline.question("Project game version (" + Object.values(GameVersion) +  "): ")
        this.config.gameVersions = (projectGameVersions !== "") ? projectGameVersions.split(",") : this.config.gameVersions
        let projectRepository = readline.question("Project repository: ")
        this.config.repository = (projectRepository !== "") ? projectRepository : this.config.repository
        let projectLicense = readline.question("Project license: ")
        this.config.license = (projectLicense !== "") ? projectLicense : this.config.license
        this.logger.info("Writing wowlua.json...")
        fs.writeFileSync("wowlua.json", JSON.stringify(this.config, null, 4))
        fs.writeFileSync(".gitignore", "libs")*/
        this.logger.info("Initialized wowlua project!")
    }

}