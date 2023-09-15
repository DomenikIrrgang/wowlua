import { Command } from "cli-program-lib/decorators/command.decorator";
import { Inject } from "cli-program-lib/decorators/inject.decorator";
import * as svn from "node-svn-ultimate"
import { simpleGit } from "simple-git"
import { Dependency, DependencyType, WowluaConfig } from "../config/wowlua.config";
import { Logger } from "cli-program-lib/logging/logger";
import * as fs from "fs";

@Command({
    name: "install",
    description: "Adds and downloads all dependencies to the project"
})
export class InstallCommand {

    @Inject(WowluaConfig)
    private config: WowluaConfig;

    @Inject(Logger)
    private logger: Logger;

    public async execute(args: String[]): Promise<void> {
        this.logger.debug("Installing dependencies");
        if (args[3] !== undefined) {
            const dependency = this.config.dependencies.find((dependency) => dependency.name === args[3])
            if (dependency === undefined) {
                throw new Error("Dependency " + args[3] + " not found")
            }
            await this.installDependency(dependency)
        } else {
            for (let dependency of this.config.dependencies) {
                await this.installDependency(dependency);
            }
        }
    }

    private async installDependency(dependency: Dependency): Promise<void> {
        switch (dependency.type) {
            case DependencyType.SVN:
                this.logger.debug("Installing dependency " + dependency.source + " from SVN");
                return this.installSVN(dependency);
            case DependencyType.GIT:
                this.logger.debug("Installing dependency " + dependency.source + " from GIT");
                return this.installGIT(dependency);
            case DependencyType.LOCAL:
                this.logger.debug("Installing dependency " + dependency.source + " from local");
                return this.installLocal(dependency);
            default:
                throw new Error("Unknown dependency type " + dependency.type);
        }
    }

    private async installSVN(dependency: Dependency): Promise<void> {
        this.uninstallDependency(dependency)
        return new Promise((resolve, reject) => {
            const url = dependency.source + ((dependency.path !== undefined) ? "/" + dependency.path : "")
            const targetPath = this.config.libPath + ((dependency.path !== undefined) ? "/" + dependency.path : "")
            svn.commands.checkout(url, targetPath, { revision: (dependency.version !== undefined) ? dependency.version : "HEAD" }, (error: string) => {
                if (error) {
                    throw new Error("Error while installing dependency " + dependency.name + " from SVN: " + error)
                }
            });
            resolve();
        });
    }

    private async installGIT(dependency: Dependency): Promise<void> {
        const installPath = this.config.getLibPath() + "/" + dependency.source.split("/").pop()
        if (fs.existsSync(installPath)) {
            fs.rmSync(installPath, { recursive: true })
        }
        return new Promise((resolve, reject) => {
            const url = dependency.source
            const targetPath = this.config.getLibPath()
            const git = simpleGit({
                baseDir: targetPath,
            })
            const deleteNonPathDirectries = (dependency: Dependency) => {
                fs.cpSync(installPath + "/" + dependency.path, targetPath + "/" + dependency.name, { recursive: true })
                fs.rmSync(installPath, { recursive: true })
            }
            git.clone(url).then(() => {
                if (dependency.version !== undefined) {
                    const git = simpleGit({
                        baseDir: installPath,
                    })
                    git.checkout(dependency.version).then(() => {
                        deleteNonPathDirectries(dependency)
                        resolve()
                    }, (err) => reject(err))
                } else {
                    deleteNonPathDirectries(dependency)
                    resolve()
                }
            }, (err) => reject(err))
        })
    }

    private async installLocal(dependency: Dependency): Promise<void> {
        return new Promise((resolve, reject) => {
            reject("Local not implemented yet");
        });
    }

    private uninstallDependency(dependency: Dependency): void {
        const targetPath = this.config.getLibPath() + "/" + dependency.name
        if (!fs.existsSync(targetPath)) {
            return
        }
        fs.rmSync(targetPath, { recursive: true })
    }

}