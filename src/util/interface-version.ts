import { GameVersion } from "../util/game-version"

export let INTERFACE_VERSION: Map<GameVersion, string> = new Map()
INTERFACE_VERSION.set(GameVersion.CLASSIC, "11306")
INTERFACE_VERSION.set(GameVersion.WOTLK, "30300")
INTERFACE_VERSION.set(GameVersion.RETAIL, "90005")