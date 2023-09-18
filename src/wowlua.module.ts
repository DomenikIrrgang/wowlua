import { Module } from "cli-program-lib/decorators/module.decorator"
import { JsonParser } from "cli-program-lib/properties/json.parser"
import { InitCommand } from "./commands/init.command";
import { WowluaConfig } from "./config/wowlua.config";
import { BuildCommand } from "./commands/build.command";
import { InstallCommand } from "./commands/install.command";
import { Performance } from "./util/performance";
import { BuildSystem } from "./build-system/build-system";
import { DependencyManager } from "./dependency-manager/dependency-manager";
import { DependencyAdd } from "./commands/ dependency-add";
import { DependencyRemove } from "./commands/dependency-remove";

@Module({
    commands: [
        InitCommand,
        BuildCommand,
        InstallCommand,
        DependencyAdd,
        DependencyRemove
    ],
    providers: [
        WowluaConfig,
        JsonParser,
        Performance,
        BuildSystem,
        DependencyManager
    ],
})
export class WowLuaModule {}