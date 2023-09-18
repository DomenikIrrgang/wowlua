import { Command } from "cli-program-lib/decorators/command.decorator"
import { WowluaConfig } from "../config/wowlua.config"
import { Inject } from "cli-program-lib/decorators/inject.decorator"
import { Logger } from "cli-program-lib/logging/logger"
import { GameVersion } from "../util/game-version"
import { writeFileSyncRecursive } from "../util/file-write-sync"

@Command({
    name: "init",
    description: "Initialize a new wowlua project."
})
export class InitCommand {

    @Inject(WowluaConfig)
    private config: WowluaConfig

    @Inject(Logger)
    private logger: Logger

    private templatesPath = import.meta.dir + "/../../templates"

    public execute(... args: String[]): void {
        this.createProjectConfigFile()
        this.createGitIgnoreFile()
        this.createReadmeFile()
        this.createMainFile()
        this.logger.info("Initialized wowlua project!")
    }

    private createProjectConfigFile(): void {
        this.logger.info("Initializing wowlua project...")
        this.config.name = prompt("Project name: ", this.config.name)
        this.config.description = prompt("Project description: ", this.config.description)
        this.config.version = prompt("Project version: ", this.config.version)
        this.config.author = prompt("Project author: ", this.config.author)
        this.config.gameVersions = prompt("Project game version (" + Object.values(GameVersion) +  "): ", this.config.gameVersions.toString()).split(",") as GameVersion[]
        this.config.repository = prompt("Project repository: ", this.config.repository)
        this.config.license = prompt("Project license: ", this.config.license)
        this.logger.info("Writing wowlua.json...")
        this.config.save()
    }

    private createGitIgnoreFile(): void {
        this.logger.info("Writing .gitignore...")
        Bun.write(".gitignore", `${this.config.buildPath}\n${this.config.libPath}\n`)
    }

    private async createReadmeFile(): Promise<void> {
        this.logger.info("Writing README.md...")
        let readme = await Bun.file(this.templatesPath +  "/README.md").text()
        readme = readme.replace("{{ name }}", this.config.name)
        readme = readme.replace("{{ description }}", this.config.description)
        Bun.write("README.md", readme)
    }

    private createMainFile(): void {
        this.logger.info("Writing src/main.lua...")
        writeFileSyncRecursive("src/main.lua", `print("Hello from ${this.config.name}!")`)
    }

}