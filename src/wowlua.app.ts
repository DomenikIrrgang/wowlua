import { App } from "cli-program-lib/decorators/app.decorator"
import { Application } from "cli-program-lib/application/application"
import { WowLuaModule } from "./wowlua.module";

@App({
    bootstrap: WowLuaModule
})
export class WowluaApplication extends Application {}