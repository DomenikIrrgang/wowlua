import { Injectable } from "cli-program-lib/decorators/injectable.decorator";
import { Dependency } from "./dependency";
import { DependencyType } from "./dependency-type";
import { Logger } from "cli-program-lib/logging/logger";
import { Inject } from "cli-program-lib/decorators/inject.decorator";
import { WowluaConfig } from "../config/wowlua.config";
import simpleGit, { gitP } from "simple-git";
import * as fs from "fs";
import * as svn from "node-svn-ultimate"
import { readDirSyncRecursive } from "../util/ready-directory-recursive";

@Injectable()
export class DependencyManager {

    @Inject(WowluaConfig)
    private config: WowluaConfig;

    @Inject(Logger)
    private logger: Logger;

    public async installAllDependencies(): Promise<void> {
        this.logger.info("Installing dependencies");
        for (let dependency of this.config.dependencies) {
            await this.installDependency(dependency);
        }
    }

    public async installDependency(dependency: Dependency): Promise<void> {
        this.uninstallDependency(dependency)
        switch (dependency.type) {
            case DependencyType.SVN:
                this.logger.info("Installing dependency " + dependency.name + " from SVN");
                return this.installSVN(dependency);
            case DependencyType.GIT:
                this.logger.info("Installing dependency " + dependency.name + " from GIT");
                return this.installGIT(dependency);
            case DependencyType.LOCAL:
                this.logger.info("Installing dependency " + dependency.name + " from local");
                return this.installLocal(dependency);
            default:
                throw new Error("Unknown dependency type " + dependency.type);
        }
    }

    private async installSVN(dependency: Dependency): Promise<void> {
        return new Promise((resolve, reject) => {
            const url = dependency.source + ((dependency.path !== undefined) ? "/" + dependency.path : "")
            const targetPath = this.config.libPath + "/" + dependency.name + ((dependency.path !== undefined) ? "/" + dependency.path : "")
            svn.commands.checkout(url, targetPath, { revision: (dependency.version !== undefined) ? dependency.version : "HEAD" }, (error: string) => {
                if (error) {
                    this.logger.error("Error while installing dependency " + dependency.name + " from SVN: " + error)
                }
            });
            resolve();
        });
    }

    private async installGIT(dependency: Dependency): Promise<void> {
        const url = dependency.source
        const targetPath = this.config.libPath + "/" + dependency.name + ((dependency.path !== undefined) ? "/" + dependency.path : "")
        const installPath = this.config.getLibPath()
        const gitPath =  installPath + "/" + url.split("/").pop()
        if (fs.existsSync(installPath + "/" + url.split("/").pop())) {
            fs.rmSync(installPath + "/" + url.split("/").pop(), { recursive: true })
        }
        if (fs.existsSync(targetPath)) {
            fs.rmSync(targetPath, { recursive: true })
        }
        fs.mkdirSync(targetPath, { recursive: true })
        const git = simpleGit({
            baseDir: installPath,
        })
        const deleteNonPathDirectries = (dependency: Dependency) => {
            if (fs.existsSync(installPath + "/" + dependency.path)) {
                fs.cpSync(gitPath + "/" + dependency.path, targetPath, { recursive: true })
            } else {
                this.logger.error("Dependency source of '" + dependency.name + "' " + dependency.source + " does not contain path '" + dependency.path + "'")
            }
            fs.rmSync(gitPath, { recursive: true })
        }
        try {
            await git.clone(url)
            if (dependency.version !== undefined) {
                await git.raw(["checkout", "--force", dependency.version])
                deleteNonPathDirectries(dependency)
            }
        } catch (error) {
            this.logger.error("error cloning", error)
        }
        
    }

    private async installLocal(dependency: Dependency): Promise<void> {
        return new Promise((resolve, reject) => {
            reject("Local not implemented yet");
        });
    }

    public uninstallDependency(dependency: Dependency): void {
        this.logger.info("Uninstalling dependency " + dependency.name)
        let targetPath = (this.config.getLibPath() + "/" + dependency.name)
        if (targetPath.endsWith("/")) {
            targetPath = targetPath.substring(0, targetPath.length - 1)
        }
        if (!fs.existsSync(targetPath)) {
            return
        }
        fs.rmSync(targetPath, { recursive: true })
    }

    public addDependency(dependency: Dependency): void {
        if (!this.hasDependency(dependency)) {
            this.config.dependencies.push(dependency)
            this.config.save()
        } else {
            this.updateDependency(dependency)
        }
    }

    public removeDependency(dependency: Dependency): void {
        this.logger.info("Removing dependency " + dependency.name)
        this.config.dependencies = this.config.dependencies.filter((d) => d.name !== dependency.name)
        this.config.save()
    }

    private hasDependency(dependency: Dependency): boolean {
        return this.config.dependencies.find((d) => d.name === dependency.name) !== undefined
    }

    private updateDependency(dependency: Dependency): void {
        if (this.hasDependency(dependency)) {
            this.removeDependency(dependency)
        }
        this.addDependency(dependency)
    }

}