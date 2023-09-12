import { GameVersion } from "../util/game-version"

export let TOC_NAME: Map<GameVersion, string> = new Map()
TOC_NAME.set(GameVersion.CLASSIC, "Classic")
TOC_NAME.set(GameVersion.WOTLK, "Wrath")
TOC_NAME.set(GameVersion.RETAIL, "Mainline")