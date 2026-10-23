# Changelog

All notable changes to wowlua. Each minor version closes a milestone of the [roadmap](spec/roadmap.md).

## Unreleased

### New

- **Debugger**: `wowlua debug` lets VS Code (or any editor that speaks the Debug Adapter Protocol) debug a test file or your addon running in the emulated client: line breakpoints with conditions, hit counts and log messages; breakpoints on functions, API calls (`C_Timer.After`), events (`event:PLAYER_LOGIN`), scripts (`script:MyFrame:OnClick`) and timers; stop on errors, on taint or on a blocked protected action. Step into event handlers, `OnUpdate` and timers, and out of one handler into the next. The call stack shows which event or script ran your code, variables show their taint and secret values what they hide, widgets their size, points and children, and watches can change locals.
- **Screenshots**: `expect(frame).toMatchScreenshot()` (or `expect(mock.screen)`) compares what the player would see with an image stored in `__screenshots__` next to the test; a missing one is written, `wowlua test -u` updates it, and a mismatch writes the new image and a diff beside it. `mock.screenshot()` and `wowlua screenshot tests/x.test.wowlua -o x.png` write images too. Drawn with your own WoW installation's textures and fonts when there is one (for camelot, retail and every classic client), never copied anywhere.
- **For AI agents**: `wowlua mcp` is an MCP server: an agent can start your addon in the emulated client, look at it in screenshots, find frames, click, type, run slash commands, move time on, set breakpoints, step, read variables and profile it.
- **3D models in screenshots**: model frames and model scenes show their models, read from your WoW installation: `SetDisplayInfo`, `SetCreature`, `SetModel` with a file ID, and an actor's `SetModelByCreatureDisplayID` and `SetModelByFileID`, posed in their animation, turned by `SetFacing`, zoomed by `SetPortraitZoom`, placed by the scene's camera. Characters too (`SetUnit`, `SetModelByUnit`, `TryOn`): race, skin, face and hair, and what they wear, so the default UI's character sheet shows your character; humanoid NPCs wear their gear. With an installation at hand, `C_ModelInfo` answers from the client's model scene tables, so Blizzard's model scenes place their models: the character sheet, the dressing room (`DressUpItemLink`) and the wardrobe show your character. `SetPortraitTexture` draws the unit's head in 3D, as unit frames show it in game. `SetCreature` finds creatures you have met (the client's creature cache), `SetItem` shows an item's own model, and models show their particles (with their tails, falling and changing texture cells over their life), glows, reflections, trails behind moving parts (ribbons) and parts that always face the camera.
- **Minimap in screenshots**: the minimap shows the land around the player from your WoW installation's map tiles, at the minimap's zoom, through its mask, with the player's arrow in the middle. It follows the player: `mock.moveTo({ mapID = 1429, x = 0.42, y = 0.65 })` and `Minimap:SetZoom(2)` show Goldshire close up. `mock.moveTo({ facing = math.pi / 2 })` turns the player west (`GetPlayerFacing`): the arrow turns, or with the rotating minimap (`rotateMinimap`) the map does. Group members you place with `mock.spawnUnit({ …, world = { y, x } })` show as their class's dots, and `Minimap:PingLocation(x, y)` shows a ping (`GetPingPosition`, `MINIMAP_PING`). Indoors the minimap stays empty for now.
- **Profiler**: `wowlua debug` and `wowlua mcp` profile calls, instructions run, memory allocated and time per function, event, script, `OnUpdate` and game frame, and write speedscope files (speedscope.app opens them); `wowlua test --profile-out prof` writes one for each test file.
- A profile can leave files out of its builds: `"profiles": { "release": { "exclude": ["src/Diagnostics/**"] } }` keeps developer tools in your dev builds and tests and out of the release players get. Guard an import of such a file in code you keep with `@if(build.debug)`.
- A script handler set on an instance of your widget class gets that class: in `toggle:SetScript("OnClick", function(self) … end)` with `class Toggle extends Button`, `self` is a `Toggle`, so its fields are typed (it was a `Button`). The same goes for every method of the game's classes that takes or gives `Self`.
- `keyof T` and `T[K]` types, as in TypeScript: `function get<K extends keyof Defaults>(key: K): Defaults[K]` gives each setting its own type, and a key that isn't a field of `Defaults` is an error (E2014).
- `SetScript` and `HookScript` know what the game passes each script: in `frame:SetScript("OnUpdate", function(self, elapsed) … end)`, `self` is the frame and `elapsed` a number, for every script with known arguments (`OnClick`, `OnEnter`, `OnValueChanged`, `OnKeyDown`, …).
- W2063 reports values that are `any` without anyone writing it: a parameter nothing gives a type, or a local whose value is `any`. It is off unless you turn it on (`"check": { "lints": { "W2063": "warn" } }`), for projects that want `any` only where it is written.
- `EventArgs<E>` works in your own functions, not only in `wowlua:events`: `function on<E extends EventName>(event: E, fn: (name: E, EventArgs<E>...) -> ())` gives `on("UNIT_HEALTH", function(name, unit) … end)` the event's argument types, and `function send<M extends MessageName>(name: M, ...: EventArgs<M>...)` checks the values sent against `declare message`. Before, it was `any`.
- A widget class can extend `FontString`, `Texture`, `Line` or `MaskTexture`: build it on the region its frame makes with `MyLabel.adopt(frame:CreateFontString(), …)`.
- `MyButton.adopt(frame, …)` builds a widget class on a frame that already exists, such as the buttons a secure group header makes, instead of creating a new one: your constructor runs on that frame and your `@script` handlers are set.
- `@deferred` on an interface method's callback parameter (`function on(name: string, @deferred fn: Handler)`) tells the load-order check that every implementation calls it later, so a callback registered through that interface at load time no longer makes files depend on each other.
- `wowlua:reflect` is typed as documented: `reflect.isInstance(v, Dog)` narrows `v` to `Dog`, `reflect.classOf(dog)` gives `ClassOf<Dog>?`, and `reflect.enumFrom(Direction, raw)` gives `Direction?` (`reflect.enumEntries` gives the members as `Direction`). Before, they were `any` and `boolean`. Your own generic functions can do the same: a parameter `ClassOf<T>` now takes `T` from the class or enum passed.
- **Parameter properties**: a constructor's parameter can declare a field, as in TypeScript: `constructor(private sink: Sink = printer, public readonly name: string)` makes `self.sink` and `self.name` and sets them from the arguments. A field declared twice is now an error (E3046).
- `mock.setPlayer(data, { events = true })` and `mock.setRenown(faction, level, { events = true })` fire the events a real change would (`PLAYER_LEVEL_CHANGED`, `UNIT_LEVEL`, `UNIT_HEALTH`, `MAJOR_FACTION_RENOWN_LEVEL_CHANGED`, …), so Blizzard's UI and your addon see the new level or renown. Without the option they stay quiet, as before.

### Changed

- The HTML coverage report (`wowlua test --coverage --reporter html`) is a page to work from: the files to test next, a file table you can sort and filter, and one file at a time with the lines that never ran (jump between them with `j` and `k`), its functions and how often each was called, and its source with each branch's counts in words. It carries wowlua's mark and colours, and is less than half the size it was.
- A generic function keeps the type of a value you pass it when that value already has one: `assert(tokens[k])` with `tokens: { [string]: Token }` is a `Token`, not a `string`. A literal written as the argument still widens (`id("x")` is a `string`).
- A type parameter with a constraint keeps a literal argument: `function f<K extends "a" | "b">(k: K): K` gives `"a"` for `f("a")`, where it gave `string` and reported that `string` doesn't satisfy the constraint.
- An error naming a very long union (such as `EventName`) shows its first members and how many more there are, instead of all of them.
- A local without a type annotation that a function changes keeps a type inside functions: `local count = 0` read in a callback that does `count = count + 1` is a `number`, a forward-declared `local update` is the function assigned to it later, and a flag saved and restored is a `boolean`. Before, they were `any` there, so mistakes with them went unreported; now they are checked, and W2063 no longer reports them.

- In a class's method or constructor, a function whose parameter is named `self` (`frame:SetScript("OnEnter", function(self) … end)`) now warns (W1004): inside it, `self` is the frame, not your instance, and `self.field` reads the wrong table. Name the parameter differently. Plain `function M:m()` code is unchanged.
- `assert` returns every argument it was given, as Lua's does, so `return assert(x, "message")` in a function that returns one value is now reported (too many values returned): it also handed the message to the caller, as a second value. Write `return (assert(x, "message"))`.
- A parameter's default can't read that parameter or a later one: in the Lua it compiles to, every parameter is in scope there and not set yet, so `function f(conf: Conf = conf)` or `constructor(private settings: Store = settings.store)` read `nil`, not the outer value wowlua checked them against. That is now an error (E1004); give the parameter another name. A function in a default may still use any parameter.
- The load-order check follows `MyClass.new(…)`: what the constructor (its default arguments included) and the field initializers read while a file loads now orders the files. Before, a default instance built at load could run before the module it reads, inside an import cycle, and fail in game with no warning.
- The load-order check follows your classes: a method called on `self`, or on an instance you made with `MyClass.new(…)` in the same file, is that class's method (or a subclass's override), so callbacks passed through your own classes no longer count as called at load. Before, such calls were unknown and could report a dependency cycle (E1014) or W1034 that wasn't real.
- When a generic call's result doesn't fit where it goes, the error names the call's type alone: `local b: boolean = id(5)` reports `number?`, not `nil | boolean | number`.
- A `readonly` field can only be assigned in its initializer or in its own class's constructor, as documented: assigning it in a method, or in a subclass's constructor, is now an error (E2010).
- A method replaced on a widget class after it is defined (`MyButton.refresh = newRefresh`, as a plugin might) now reaches the frames the class builds afterwards, its subclasses' too, as a plain class's instances see it. Frames built before keep the method they were given.

### Fixed

- Talent points follow the character's level: one a level from level 10 (earlier with Legacy's "Talented" ranks), at most 51, so the talent window and `C_Traits.GetTreeCurrencyInfo` show what a character of that level has.
- The default world's talent tree shows its talents: the player's talent config is the one the emulated client holds talents for (it named another, so Blizzard's talents tab drew only its backgrounds).
- With your WoW installation at hand, an item your test doesn't define is the game's own: `C_Item.GetItemInfo`, `DoesItemExistByID` and Blizzard's item loading answer with its name, quality, icon and class from the client's tables, so Blizzard's Legacy rewards panel no longer raises "invalid itemID". Items you define stay yours.
- Textures in XML without anchors fill their frame even when they have a size, sliders place their thumb by their value, and a texture's `rotation` attribute turns it: Blizzard's group finder, Settings' sliders and the group finder's divider look as in game.
- `C_SpecializationInfo.HasPlayerEarnedATalentPoint()` follows the player's level (from level 10, earlier with Legacy's "Talented" ranks), as in game: the talents button turns on once the character reaches it.
- `frame:GetScaledRect()` gives the rectangle times the frame's effective scale, as in game. Layouts built on it (Blizzard's action bars, `ResizeLayoutFrame`) came out too large by one over UIParent's scale, so the default UI's bars sat apart.
- A button's normal, pushed, disabled and highlight textures that `SetNormalAtlas` or `SetNormalTexture` makes fill the button, as in game; they had no size (the default UI's micro menu buttons showed no icons).
- `<MaskedTextures>` in XML masks the textures it names (`<MaskedTexture childKey="icon"/>`): action buttons' icons are masked and drawn over their slot art.
- `bar:SetStatusBarTexture("atlas-name")` takes an atlas by its name, as Blizzard's bars use it (the experience bar).
- A frame in XML with both `setAllPoints="true"` and its own `<Anchors>` is placed by its anchors and size, as in game: the player frame's resting "Z" sits at the portrait's top right instead of over the health bar.
- Textures snap to the pixel grid unless `SetSnapToPixelGrid(false)` or `snapToPixelGrid="false"` says otherwise, as in game: `IsSnappingToPixelGrid()` answers true at first, and screenshots draw their edges on whole pixels. XML's `horizTile`, `vertTile` and `texelSnappingBias` attributes are read too.

- Text with a named colour (`|cnNAME:text|r`, as chat links write it) shows and measures only its text; it showed part of the colour's name.
- `font:CopyFontObject(other)` copies the other font's file, height, colours and shadow, as in game: Blizzard's chat frames lost their font when they made it their own.
- An animation group with `setToFinalAlpha` keeps its alpha animations' end values when it finishes, so `GetAlpha` on their regions reads them, as in game; `flipBookRows`, `flipBookColumns` and `flipBookFrames` in XML are kept too.
- `GetBindingText(key, 1)` shortens the modifiers as the action bars show them (`s-1`, `c-2`), so Blizzard's hotkey labels in tests and screenshots read as in game.
- Textures from XML templates (`UIPanelButtonTemplate`'s and others') now get their `<TexCoords>` and `alphaMode`, as in game: `GetTexCoord` and `GetBlendMode` give what the template sets.
- The HTML coverage report of a workspace showed no source and took the name of the last addon in it: it now shows each file's source and the root package's name.
- Code in a branch your build removes (`if build.debug then … end` in a release, `if flavor == "vanilla" then … end` in another flavor) is no longer checked, as documented: it may use what that build doesn't have, such as an import under `@if(build.debug)`.
- `fontString:SetFormattedText(format, ...)` accepts secret values among its arguments, as the game does (a target's name in a dungeon).
- A field added to a table inside another (`API.UI.Button = button` after `API.UI = { … }`) is now part of the table other files import: they read it typed, where they got `any`, and a misspelled field there is an error.
- `for k, v in pairs(t)` over a record type with an `unknown` indexer (`{ name?: string, [string]: unknown }`) is no longer refused (E2001).
- Strict mode: a field your constructor sets in every branch of an `if … else … end` counts as set (E2052 said it wasn't), as does one set in a `do` block.
- Strict mode: after `self.icon = parent:CreateTexture()`, calling a method on the field (`self.icon:SetTexture(…)`) no longer makes the next line's `self.icon` "may be nil" again. A call that gets a field's table can't replace the field; only calls that get the table holding it forget what you set.
- A WoW function with several forms (`SetTexCoord`) accepts a last argument that gives several values, as Lua passes them: `tex:SetTexCoord(unpack(coords))` was refused (E2012).
- `Texture:GetTexture()` can give a number, the file ID of a texture set by one, as in the game; it was typed as a path only.
- A cast of an optional value to a supertype of what it holds (`w.tabs :: Button[]` for a `TabButton[]?`) is no longer refused (E2015): a cast now also compares the value's type with its `nil` left out.
- E2052 (a field not assigned in the constructor) says so when the field is assigned, but only after a call of one of the class's own methods on `self`, and names that call; before, it said the field was not assigned at all.

## 0.9.7 (2026-10-23)

### New

- A new warning, W2062: reading more values than a function of yours can give, which are always `nil`. wowlua counts what each of its `return`s gives, even when it is declared `...any`, and warns on `local a, b, c = f()` or `select(5, f())` past that. It checks your `local function`s, your module functions, the functions you import from your other files, and the methods of a file-level table (`function M:Load()`) called as `M:Load()`, `M.Load()` or `self:Load()`, wherever in the file they are defined. A common case is a `pcall` helper that returns `a, b, c, d, e, f, g, h`: values 9 and up are lost. To pass every value on, return them through `...`. It found two such bugs in SuuUI.
- Tests: `mock.drag(frame, dx, dy, frames)` drags over that many frames, so code that follows the mouse in `OnUpdate` while dragging runs as it would in the game. The mouse now starts on the dragged frame and moves with it (`GetCursorPosition`).
- Tests: `anything` and `ofType("table")` from `wowlua:test` match any value (but `nil`), or any value of a type, inside `toEqual` and `toHaveBeenCalledWith`: `expect(onShow).toHaveBeenCalledWith(anything, "TOOLTIP")`.
- Tests: the encounter timeline keeps your addon's timers (`C_EncounterTimeline.AddScriptEvent`), as WoW Forever does even with its own timeline off: they are listed, readable (`GetEventInfo`, `GetEventState`, `GetEventTimeRemaining`), can be paused, resumed, finished or cancelled, and end when their time runs out, with the timeline's events.
- Tests: `mock.postLFGListing(createData)` lists your character in the group finder, as the game's own listing panel does (the listing arrives a moment later with `LFG_LIST_ACTIVE_ENTRY_UPDATE`), and `mock.delistLFG()` takes it down again. Your addon's own `C_LFGList.CreateListing` is blocked, as in the game.
- Tests: put a group member or a pet on a creature's threat list with `mob:setThreat(value, member)`. `UnitThreatSituation` and `UnitDetailedThreatSituation` then give each of them their status and percentages as the game does: the one the creature attacks holds it, and someone above them is about to pull. `UNIT_THREAT_SITUATION_UPDATE` tells each change. Before, only your own threat could be set, always as the holder.

### Fixed

- Tests: a frame anchored upside down (its top below its bottom, or its right edge left of its left one) has a size of 0, as in the game, instead of a negative size.
- Tests: a part stack dropped on a full stack of the same item swaps places with it, as in the game; before, nothing happened.
- Tests: an accepted quest is watched only while the "auto quest watch" setting (`autoQuestWatch`) is on, as in the game, and a watch you remove stays removed.
- Tests: `C_GuildInfo.GetMOTD()` (also named `GetGuildRosterMOTD()`) from your addon in combat is blocked, as in the game ("Interface action failed because of an AddOn"). Read the message of the day before the fight, or from `GUILD_MOTD`.
- Tests: chat windows keep what Blizzard's chat code or your addon sets (`SetChatWindowName`, `SetChatWindowSize`, colour, alpha, shown, locked, docked, saved position and size), and `GetChatWindowInfo` answers it, as in the game. Before, the setters did nothing.
- A function with two parameters of the same name and types (`function(_: number, _: string)`) no longer fails its runtime check on correct calls: the first `_` was checked against the second's value.
- `wowlua test --coverage` no longer reports the closing line of a multi-line `return f({ … })` as not run.
- `GetCursorInfo()` gives the cursor's kind (`"item"`, `"spell"`, …) and its details to the checker. It was declared without results, so strict code had to read it through `any`.
- Tests: `ClearPoint("TOPLEFT")` removes just that point, as in the game. Before, it did nothing, so a Blizzard dropdown menu that opens upward near the bottom of the screen kept both points and came out upside down.
- Tests: `IsMouseOver` measures the mouse against the frame, as in the game: its offsets (`IsMouseOver(12, -12, -24, 24)`) grow or shrink the area, and a child sticking out of its parent is not over the parent. `mock.hover(frame)` and `mock.click` put the mouse on the frame (`GetCursorPosition` follows), and `mock.hover(nil)` takes it off every frame. Before, `IsMouseOver` only asked whether the hovered frame was the frame or one of its children.
- Tests: an item put on a part stack of the same item in your bags (`C_Container.PickupContainerItem` onto it) stacks onto it, as in the game; whatever doesn't fit stays where it was. It used to stay on the cursor, so a bag sort that puts part stacks together failed in tests.
- Tests: a frame that holds a secure (protected) frame is protected too, as in the game: `IsProtected()` gives `true, false`, showing, hiding or moving it from your addon in combat is blocked, and your secure snippets may resize or move it in combat (Blizzard's spells window among them). It used to count as a plain frame.
- Tests: `C_SpellBook.PickupSpellBookItem` from your addon in combat is blocked ("Interface action failed"), as in the game; out of combat it still picks the spell up. To let players drag spells in combat, pick them up through Blizzard's secure drag template (`SecureHandlerDragTemplate` with an `_ondragstart` that returns `"spell", spellID`).
- Tests: anchoring a secure (protected) frame to a texture or a font string fails with the game's error ("Cannot anchor protected frames to regions"), as it does in the game even out of combat. It used to work in tests.
- Tests: a frame whose alpha was set from a secret value keeps reading its alpha back as a secret, as in the game, even after you set a plain alpha. Code that reads `GetAlpha()` again later (a fade from the current alpha) now fails in the test as it does in the game.
- `wowlua test --coverage` with a filter (`wowlua test --coverage Units/Threat`) no longer fails on the coverage minimums in `wowlua.json`: they are for the whole suite, and a filtered run's totals still count every file. It used to fail even when all its tests passed.
- Tests: a group member logging in or out (`member:setOnline(false)`) now sends `UNIT_CONNECTION`, for their target token too, as the game does. Only the roster update came before.
- Tests: a spell your character learns from a trainer is known afterwards, as in the game: `IsPlayerSpell` and `C_SpellBook.IsSpellKnown` say so. They kept answering as before the training.
- Tests: your addon's tooltips show in combat, as in the game. `GameTooltip:SetUnit`, `SetSpellByID`, `SetItemByID` and the other setters gave an empty tooltip in a fight because `C_RestrictedActions.CheckAllowProtectedFunctions` refused your addon on any frame; it now refuses only on a protected one (a secure button, UIParent).
- Tests: an edit box lets go of the keyboard when it is hidden (or a parent of it is), as in the game. It kept the focus, so keys pressed later in the test went into the hidden box instead of opening windows with their key bindings.

## 0.9.6 (2026-10-22)

### New

- **Collections in tests**: `mock.setMounts`, `mock.setToys` and `mock.setAppearances` give the account its mounts, toys and collected appearances, and a world profile can hold them (`mounts`, `toys`, `appearances`). The mount journal, the toy box and the wardrobe then answer from them as the game does (their filters, search, favourites and events, recorded from the game), so a collections window can be tested without stubbing every journal call. `/stdtest snapshot` records them from your character.
- Tests: choose the machine the game runs on with `"test": { "platform": "windows" }` in `wowlua.json`, or `config({ platform = "windows" })` in one test file. Key names (`GetBindingText`: "Delete" on Windows, "Del" on a Mac), graphics and sound answers and `IsMacClient`/`IsWindowsClient` follow it. The Mac stays the default.
- Tests: choose the screen the game draws on with `"test": { "screen": { "width": 2560, "height": 1440 } }` in `wowlua.json`, or `config({ screen = { ... } })` in one test file, with an optional `dpiScale` and `uiScale` ("Use UI Scale" on). UIParent's size and scale, `GetScreenWidth` and `GetPhysicalScreenSize` follow it. The default is 1920 by 1080.
- Tests: `mock.setField(t, key, value)` sets any field, `nil` included, until the test ends, and puts it back untainted as `mock.stub` does. Taking a frame or a game API away with an assignment instead leaves Blizzard's field tainted for the rest of the file.
- A new warning, W2061: `a and nil or b` (or `false` in the middle) always gives `b`, since the `nil` makes the `or` take over. Write an `if`, or `not a and b or nil` when `b` is never `nil` or `false`. It found three such bugs in real code.

### Changed

- Tests: `mock.drag(frame, dx, dy)` and `mock.sizeBy(grip, dw, dh)` move by screen units, as `GetCursorPosition` counts them: a distance in the frame's own units times its `GetEffectiveScale()`. Since UIParent now has the screen's UI scale (0.71 by default), a test that dragged by frame units moves the frame further than before: multiply its distances by the frame's effective scale.

### Fixed

- The dev console's error list: clicking an open error closes it again (it stayed open).
- Tests: `C_QuestLog.GetQuestsOnMap` lists the quests in your character's log, as in the game. It listed quests of the character wowlua was recorded with, which your test's character didn't have (no title, not in the log).
- Tests: a table you pass to `mock.fireEvent` or `mock.fireUnitEvent` (`{ isFullUpdate = true }`) reaches Blizzard's code as the game's own would, without your addon's taint. Blizzard's buff frame no longer errors in combat ("Auras cannot be accessed when secret while tainted") after a test fires `UNIT_AURA`.
- Tests: talents open at level 5 as in the game: `C_SpecializationInfo.CanPlayerUseTalentUI()` and `CanPlayerUseTalentSpecUI()` follow your character's level, so Blizzard's spells window opens its Talents tab for a character of level 5 and up. They stayed "available at level 5" whatever the level.
- Tests: a group made with `mock.group:add` is a group you made yourself, not a dungeon finder's: `IsInGroup(LE_PARTY_CATEGORY_INSTANCE)`, `IsInRaid(LE_PARTY_CATEGORY_INSTANCE)` and `GetNumGroupMembers(LE_PARTY_CATEGORY_INSTANCE)` say none. They ignored the category, so an addon picked `INSTANCE_CHAT` for its party messages and they were refused.
- Tests: `UnitFullName("player")` gives your realm as its second value, as in the game, so your addon recognizes its own messages (sent as `Name-Realm`). It gave `nil`.
- Tests: worn gear can be given its durability in a strictly typed test: `mock.player:equip(slot, { id, durability, maxDurability })` (the fields were read but not declared).
- Tests: a button of yours made with `SecureHandlerClickTemplate` runs its `_onclick` snippet when clicked (`mock.click`), as in the game. Blizzard's handler failed with "Invalid 'self' frame handle" and the snippet never ran.
- Tests: text is as wide and as tall as in the game. `GetStringWidth`, `GetUnboundedStringWidth`, wrapping, truncation and `SetTextToFit` measure each character with the game's own fonts (Friz Quadrata, Arial Narrow, Skurri and Morpheus), at the size the game draws them on your test's screen. They used a guess per character, so text-sized frames and wrapped lines were off.
- Tests: `UIParent:GetScale()` and every frame's `GetEffectiveScale()` include the screen's UI scale as in the game (0.71 on a 1920 by 1080 screen; it was 1). A frame that ignores its parent's scale (`SetIgnoreParentScale(true)`) or has no parent is now placed, sized and measured as in the game: one at UIParent's scale lines up with UIParent and its text is as wide.
- Tests: every atlas of the game is known, as the game has it (its own tables): `C_Texture.GetAtlasInfo` gives its size, file, texture coordinates, tiling and `rawSize`, `C_Texture.GetAtlasExists` tells, and a texture set with `useAtlasSize` takes its size. An atlas the game lacks gives nothing, as in the game; before, any atlas wowlua had not recorded was a 64 by 64 whole texture, and Blizzard's quest log errored ("Division by zero") on one sized 0.
- Tests: map positions match the game's (`C_Map.GetMapPosFromWorldPos` with an override map, Azeroth's world map, `GetWorldPosFromMapPos`), and the game's second Eastern Kingdoms and Kalimdor maps (1463, 1464) exist.
- Tests: `C_GameRules.IsGameRuleActive` answers every game rule as the game does, and the artifact ranks cost what they do in the game up to rank 10.
- Tests: some errors read as the game words them: a `CreateAbbreviateConfig` divisor that is not a power of ten, and infinity passed to a widget's number (on Windows "must be a finite number").
- Tests: `C_TransmogCollection.GetAppearanceSources(appearanceID)` without a category gives the appearance's sources, as in the game (it gave nothing), and a source has its item's name and quality once the item is loaded.
- Tests: a key bound to a button with `SetBindingClick` or `SetOverrideBindingClick` clicks it once, as in the game: only for the clicks it registered (by default the left button's release). It clicked on both the key's press and release, so a toggle opened and closed at once.
- Tests: a model's or actor's `GetItemTransmogInfoList()` has an entry for every slot, so Blizzard's dressing room no longer errors ("attempt to index local 'mainHandInfo'").
- Tests: `mock.slash` runs Blizzard's own slash commands (`/inspect` and the others the game keeps in its own lists), which it silently skipped: with Blizzard's UI the line is typed into the chat and sent, as you do in the game, so an unknown command also gets the game's help line.
- Tests: `GetText(token, gender)` gives the global string by its name (a woman's form for gender 3), as in the game; it gave nothing.
- Tests: `mock.spawnUnit` and `mock.setPlayer` take `healthMax` and `powerMax` as declared (they were ignored: the unit's maximum was its current health). The types now know every unit field the engine reads (`factionGroup`, `pvp`, `classification`, the creature's type and family) and every unit method (`joinGroup`, `loot`, `openTrade`, `setInspect`, `setThreat`, `useSpell`), so strict tests can use them.
- Tests: an inspected player's gear has its item ID, icon and quality (`GetInventoryItemID`, `GetInventoryItemTexture`, `GetInventoryItemQuality`), not only its link.
- `wowlua test --coverage` no longer crashes with "internal compiler error: stale handle" when code it counts (a Blizzard frame's or your addon's) is thrown away during the run.
- An internal compiler error prints where it happened when `RUST_BACKTRACE=1` is set, for the bug report.

## 0.9.5 (2026-10-21)

### Changed

- Fewer false warnings: a local in a test may reuse the name of a test or a `@beforeEach` function (W1004), since nothing calls those by name.
- Fewer false warnings: what your test files do to the game's API (taking `C_EventUtils` away and back, `_G[name] = nil`) no longer counts toward your addon's load order, so your addon's own reads of it are no longer W1034 or W1033.
- Fewer false warnings: the bundling warning W6001 now only flags code that takes a Lua file name from `debugstack` or `debuglocals`. Keeping a stack for an error report, or reading your addon's folder from it, works the same in a bundled build.
- Tests: an item from `mock.defineItem` is ready at once unless you give `cached = false`, a `delay` or `missing` (as the docs said: before, an item defined without options never loaded), and has item level 1 unless you give one.

### Fixed

- Tests: the auction house answers every query. A query sent while another was under way was never answered, so a search right after the house opened never came back.
- Tests: each `mock.defineAuctionHouse` starts a fresh auction house, so purchases, search results and posted auctions no longer carry over from an earlier test (two auctions could share an ID, and cancelling one removed both).
- Tests: changing an item key from the auction house (for a sell search) no longer changes the house's own data.
- Tests: a Shift-click counts as a modified click (`IsModifiedClick("CHATLINK")`), so Shift-click linking works as in the game.

## 0.9.4 (2026-10-21)

M9 so far: faster filtered test runs for large suites, found while
continuing SuuUI's unit tests on 0.9.3. The language server, the VS Code
extension and the milestones after M9 are planned; see the
[roadmap](spec/roadmap.md).

### Changed

- `wowlua test` with a filter loads only the test files whose path or source contains it (a test's name is written in its file), so a filtered run of a large suite takes seconds instead of loading every file's emulated client
- `wowlua test` runs `--jobs` test files at once when given (or `WOWLUA_JOBS`, `build.jobs`), else one a core

### Fixed

- `toHaveBeenCalledWith` lets a call leave out an optional argument the spied function takes (`spy("guildInvite")` for `(name: string, member: Member?)`), as a call of the function does, instead of E2007
- `wowlua test` with Blizzard's UI: wheeling the quest log no longer raises "Division by zero" (its bottom shadow's atlas size)

## 0.9.3 (2026-10-21)

M9 so far: what writing SuuUI's unit tests turned up in the test runner,
the mock library, the emulated client and the checker, and the emulator's
coverage report. The language server, the VS Code extension and the
milestones after M9 are planned; see the [roadmap](spec/roadmap.md).

### New

- The emulator's coverage report ([docs/emulator-coverage.md](docs/emulator-coverage.md)), generated by `cargo xtask coverage-report` and kept current by `cargo xtask check`: which API functions and widget methods the engine models, answers from recordings or from the documentation, how its answers compare with the client's, the rules still unverified, the known gaps and the in-game probes that would close them (S4)
- `wowlua test` takes several filters (`wowlua test Core/Util UI/Bar`): the files and tests matching any of them run
- `mock.group:remove(unit)` and `mock.group:leave()`; `C_PartyInfo.LeaveParty()` leaves the group a round trip later ("You leave the group.", `GROUP_LEFT`, the roster)
- E2060: `local a, b: any, any = …` (a local after an annotated one named after a type) is an error, not a third local named `any`

### Changed

- `wowlua check` type-checks the test side too (test files, mocks and helpers), as `wowlua test` compiles it
- `mock.errors()` lists the errors since the running test began (its `@beforeEach` hooks included), not every error since the file started
- `mock.click` sends the clicks the button registered for (`RegisterForClicks`, the XML's `registerForClicks`), down then up, as the client does: by default only the left button's up click, so a secure action button acting on the key down (`ActionButtonUseKeyDown`) acts
- `unit:setName` tells `UNIT_NAME_UPDATE` for the unit's tokens when the name changes (`{ quiet = true }`: not)

### Fixed

- `wowlua test` with one test file that does not compile shows its errors and runs the other files (the file fails the run unless a filter leaves it out) instead of running nothing
- `mock.fake` and `mock.stub` put the field back with its taint: a global the client set is secure again after the test, so Blizzard's code reading it later no longer runs tainted (and errors on secret values)
- The matchers compare two secrets by the values inside them (`toHaveBeenCalledWith(unit, mock.secret(true))`)
- `expect(nil).never.toBeShown()` (and the other widget matchers on a value that is not a widget) fails with "expected a frame, got nil" instead of an internal error
- `mock.stub` takes any table, widgets and objects included, without a cast
- A ticker due exactly at the end of `mock.advanceTime` runs in that advance, whatever fractions the clock moved by before
- `@cases` values are typed by the test's parameters, as arguments are (a literal fits a literal union)
- A module's local that only test files read through `import internal` is no longer reported unused (W1003)
- The emulated client: `OnSizeChanged` runs when a frame's size follows its anchors (a `SetAllPoints` child of a resized frame), `SetAlphaFromBoolean` sets the alpha, `SetVerticalScroll` and `SetHorizontalScroll` run `OnVerticalScroll` and `OnHorizontalScroll`, `SetIgnoreParentScale` works (`IsIgnoringParentScale`, the effective scale), `IsIgnoringParentAlpha` reads `SetIgnoreParentAlpha`, `SetTexture(file)` clears the texture's atlas, and a secret text's width is secret

## 0.9.2 (2026-10-20)

M9 so far: what porting SuuUI (a 137k-line UI replacement) to strict wowlua
turned up in the checker, the WoW API declarations, the standard library and
the tooling, a standard library safe with secret values throughout, and the
emulated client closer to the real one. The language server, the VS Code
extension and the milestones after M9 are planned; see the
[roadmap](spec/roadmap.md).

### New

- The emulated client answers 33,075 of the 33,727 calls the client answered in its full probe, and 25 of the 29 recorded scenarios replay line for line (combat, auradeath, inspect, legacy and mail2 among them; quest, profession, death and hearth within a few lines) (S4)
- The default camelot character leads a guild, as the recorded one does (ranks and rights, roster, bank, club); `mock.joinGuild`, `leaveGuild` and `defineGuildMember` choose another (S4)
- `/stdtest engine methodprobe` asks the client every widget method of every widget kind, as the full probe does for the API (S4)
- A field named by a string key may be optional: `{ ["not"]?: boolean }`
- A function over the 60-upvalue limit reads its excess imports through the module registry, as the spec describes, instead of failing with E5001
- `wowlua:callbacks`: `registry:off(name, handler)` cancels every subscription of a function, and `offAll(owner, name?)` only an owner's subscriptions to `name`; both checked against the event type (E8420)
- `wowlua:combat`: `defer(fn, key)`: calls with one key keep the first one's place in the queue, and only the latest function runs
- `wowlua:log`: `configure({ prefix, chatLevel })` sets the chat prefix of the default sink and `print`, and the lowest level the default sink writes to chat (`"off"`: none)
- `wowlua:secret`: `readable(v)` narrows to a present value that may be looked into (`Plain`): not nil, not a secret, and not a table whose security keeps addons out
- `wowlua:slash`: `Router:add(name, command, owner?)` (checked like the definitions, E8112), `remove`, `removeAll(owner)` and `commands()`; `router(defs, { order = "added" })` lists help in the order commands were added
- `wowlua:table`: `copy(t, { shared = false })` gives every nested table a copy of its own; `keys(t, less)` sorts by a comparator
- `wowlua:table`: a stable `sort` that never raises "invalid order function", `by` (order by a key), `order`, and the binary-search helpers `insertSorted` and `searchSorted`; secret elements and keys never reach a comparator and sort last
- `wowlua:collections`: `RingBuffer`, `Heap` and `LruCache`, which store secret values without reading them
- `wowlua:utf8`: `len`, `sub`, `truncate` and `chars` by characters, not bytes; a secret string passes `truncate` whole
- `wowlua:aura`: containers take `lazy` groups (added to the engine once `setGroupMax` gives them room), `exclude` spell IDs, a group's own `spacing`, `order` and `gap`, `setGroupFilter` and `setGroupSize` (buttons resized out of combat); `aura.hasContainers()`
- Spec: the unchecked cast (`local f: T = e :: any`) and `@unchecked` next to the runtime checks
- `wowlua:events`: `watchUnit(events, handler, owner?)`: unit events on one frame of the watch's own, pointed at other units with `setUnit(unit?, unit2?)` without subscribing again (`nil` unregisters); the handler gets the event name first
- `wowlua:events`: `offAll(owner, name?)` cancels only the owner's handlers of one event or message (a watch stops watching that event)
- `wowlua:errors`: listeners get `(msg, info)` with `info = { component?, owner?, stack? }`: the stdlib component, the owner of the failing handler (events, callback registries, hooks with `opts.owner`) and the stack, taken only when an error happens
- `wowlua:timer`: `repeating(sec, fn)`, a repeating timer with `start`, `stop`, `setRunning(running)` and `isRunning()` (stopped at first; the first tick one interval after each start)
- A string-keyed field may be read-only: `{ readonly ["end"]: number }`
- E1016: an export that redeclares a module-level name (`local items = {}` then `export function items()`, a forward-declared `local f` then `export function f()`), and a name exported twice
- W1036: a module of an import cycle that reads an export without a type annotation (or a `typeof` alias) of a module of the cycle checked after it, where it is `any`, is told to annotate it there
- W1036 also for a `typeof` alias of a later module of the cycle used through a namespace import (`local c: qn.Conf`), and for an unannotated file-level local a test file's `import internal` reads; annotated locals and functions read that way have their types

### Changed

- The standard library's tests run with the addon's code tainted, as the client runs it, so secret values refuse there what they refuse in game
- `wowlua:errors`' isolated calls, `wowlua:events`' dispatch, `wowlua:hooks`' hooks and `wowlua:timer`'s callbacks allocate nothing when the callback succeeds (an event handler's dispatch made about 550 bytes of garbage before)
- `wowlua:callbacks`: a handler cancelled during a `fire` is skipped for the rest of it (handlers added during a `fire` still wait for the next one)
- `wowlua:events`: a handler cancelled during a dispatch is skipped for the rest of it, as in `wowlua:callbacks` (handlers added during a dispatch still wait for the next one)
- `wowlua:events`: `onUnit` and `@unitEvent` take any number of units, registered in pairs on frames of their own (WoW filters a frame by two units at most); E8100 is gone
- `wowlua:table`: `copy` keeps keys as they are (a frame or a table used as a key stays itself), as Blizzard's `CopyTable` does
- `wowlua:secret`: `flag` takes any value and answers by truthiness, so legacy `1`/`nil` answers work
- `wowlua:secret`: `known<T, F>(v, fallback: F): T | F`: the fallback may be of another type, so `known(name, nil)` is a `string?`
- Generic inference binds `T` through a `T | Secret<T>` parameter (the secret members through `Secret<T>`, the rest to `T`)
- `wowlua:secure`: `state` without a function only registers the driver (for an `_onstate-<name>` snippet) and hooks nothing
- `wowlua:spell`: `charges` returns `SpellChargeInfoSecret?`, the readable table the client answers, whose counts and times may be secret on restricted clients (nil when it hands back a table addons may not index)
- A module's top-level `function f()` is a hoisted local of the module in the compiled code too, as the checker already treated it, never a global; write `_G.f = f` (or assign a declared global) for a real one
- `table` (a value `type(x) == "table"` narrowed) is assignable to every map whose values are `any`: `{ [string]: any }`, `{ [number]: any }`
- A type predicate's branch that no member of its argument reaches (a second `if not issecretvalue(c) and c then` after the first returned, the `else` of a predicate every member fits) is unreachable: the argument is `never` there and nothing in it is reported; also for `canaccessvalue`

### Fixed

- An import cycle in a package no longer makes other imports `any`: standard-library modules, library packages and the modules a cycle imports keep their types, and the modules of a cycle see each other's annotated exports
- `wowlua test` names the addon's files, its test files and the runtime as the client does (`Interface/AddOns/<Name>/src/x.lua`), so `debugstack()`, `GetSourceLocation()` and error messages show the client's names; the reports still show source paths
- W4306's help names where a wowlua library's license comes from (a `LICENSE` file or the package's `license`), not only `lua-library.license-files`
- `wowlua publish` says when it reads a token `wowlua login` saved, instead of waiting without a word while the OS credential store asks to allow it
- `wowlua publish` names the packaged zips as soon as packaging finishes, not after the uploads
- `wowlua publish` without a terminal (standard input or standard error not one) never waits for an OS credential store dialog: on every platform it then reads tokens only from the environment and `config.json`, and E4201 says why and how to fix it
- `wowlua login` and `logout` without a terminal save and remove tokens in `config.json`, where a publish without a terminal finds them, instead of the OS credential store
- `wowlua test` with Blizzard's UI: an aura container (`wowlua:aura`'s `container`, or Blizzard's `AuraContainer`) made while the addon loads takes its groups without errors; the intrinsic type's scripts no longer compile as the loading addon's code, so `AddAuraGroup` no longer fails on `CancelSignal` and a secret `SetSize`
- `wowlua test` with Blizzard's UI: a Blizzard addon the addon loads with `C_AddOns.LoadAddOn` while it loads is Blizzard's secure code, as when a test loads it, not the addon's (its functions were tainted and its files named inside the addon's folder); Blizzard's scripts compile untainted whoever's code makes them
- A wowlua library inside a workspace (a member, or a path dependency in its folder) without a license of its own has the workspace root's (its `LICENSE` file or package `license`), so no W4306
- A wowlua library path dependency in a sub-folder of a package outside a workspace has the package's license too (the package counts as its own workspace)
- `export const x: T = { … }` and an annotated plain `const` have the annotation's type, not the literal's
- An exported table has the fields assigned to it after its declaration (`function chat.Hide() … end`), sealed, for importers
- A callback taking a wider parameter type no longer widens a type argument (`table.sort(rows, function(a: { ms: number }, …) … end)`)
- Namespaces, enum tables, class tables and instances are tables for `{ [any]: any }`, so `pairs(Enum.X)` and `pairs(C_X or {})` check
- A list literal fits a record with a numeric indexer and optional named fields (`{ [number]: any, talent?: boolean }`)
- An array value fits a record whose named fields are all optional and whose numeric indexer takes its elements, also in an intersection (`any[] & { jewel?: boolean }`)
- `type X = typeof(v)` is the type of `v` in the file's later annotations, for importers and for a library's consumers, instead of `any`; a type-only import of such a module orders the check
- A declaration `@if` removes no longer hides a kept declaration of the same name in its scope (alternatives for different builds, a hoisted export)
- A table's own method replaces the one `setmetatable`'s `__index` gives it
- Reading or writing a field of a function, number or boolean is E2003 with any key, not only a `.` read
- No false W1003 for a forward-declared local that a module's `function name()` defines
- E2001 names the missing or mismatched field of a record (and hints at `x?: T`), and the first missing return value; E2015 names the union member that does not fit; a function passed to `pcall` with too few arguments is E2007 naming the argument
- `canaccessvalue(v)` narrows as the complement of `issecretvalue`: its true branch drops `Secret<…>`, in an `if` and in an `and` chain
- `cond and x` with a boolean `cond` is `false | x` in an inferred local or result, so `if y then` leaves `x`, not `true | x`; so are the results `pcall` passes on (`local ok, r = pcall(f)`)
- A generic predicate (`plain<V>(v: V | Secret<V> | nil): v is V`) infers its type parameters from the argument and narrows to them
- A declared local narrowed before a loop (`n = 0` on a `number?`) keeps its narrowing in the loop when the body keeps it; `break` paths count after the loop
- Assigning a field through a narrowed annotated local (`if item then item.x = 1 end` on an `Item | false`) checks against the members it can be, not `false`
- E2005 on a local a closure reads says when the closure sees it as declared because it is assigned after its declaration (`opts = opts or {}`); E2015 on a possibly secret value points to `canaccessvalue` and `issecretvalue`
- `wowlua:aura`: `setGroupSpells` keeps the group's dispel types
- WoW API declarations: big integers (auction house prices, `GetCancelCost`, bank tab costs, order IDs) and UI sizes (`C_NamePlate.SetNamePlateSize`) are numbers, not strings
- WoW API declarations: `issecrettable`, `canaccessallvalues`, `mapvalues` and the other secret-value functions take any value, not a function
- WoW API declarations: a structure a possibly secret function returns (`C_Spell.GetSpellCooldown`, `C_ActionBar.GetActionCooldown`, charges, loss of control) is a readable table with possibly secret fields, as the client answers, not a possibly secret table: its companion `<Name>Secret` (`SpellCooldownInfoSecret`), so functions that never answer secrets keep the plain structure (`C_Spell.GetItemCooldown`, `C_LossOfControl.GetActiveLossOfControlData`)
- WoW API declarations: functions the annotations know only by name (`SelectActiveQuest`, `GetQuestLogCompletionText`, …) take and answer any values instead of none
- WoW API declarations: `GameTooltip`'s data methods (`SetInboxItem(mailIndex, attachmentIndex)`, `SetSendMailItem(index)`, …) take the parameters of their `C_TooltipInfo` function; widget methods the annotations give no results get Blizzard's documented ones (`Minimap:GetZoom()`)
- WoW API declarations: optional `hasVoice` of `JoinChannelByName`, `exclusive` of `SetTrainerServiceTypeFilter` and `isPet` of `IsSpellKnown`; `SendChatMessage`'s target takes a channel number; `C_Traits.GetConditionInfo` has its UI wrapper's parameters and `tooltipText`
- WoW API declarations: camelot declares the client's own number globals (`LE_PARTY_CATEGORY_INSTANCE`, `NUM_LE_*`, …), and globals the annotations also name as a class (`ItemLocation`, `NamePlateDriverFrame`)
- `wowlua:errors`: `call` and `wrap` are generic as the spec declares them, so `errors.call(f, …)` checks the arguments against `f` and returns its results typed, not `any`
- `wowlua:errors`: debug builds append the module ID to a report (`[core/cache]`, before the component), as the spec says
- `wowlua:events`: `onUnit` of an event the client lacks (shared flavor mode) no longer raises; its handler is never called, as with `on`
- WoW API declarations: `UnitName`, `UnitFullName`, `UnitNameUnmodified` and `UnitNameFromGUID` answer optional names and realms (`nil, nil` for a unit that does not exist)
- WoW API declarations: named frames of Blizzard's XML are typed by their mixins and widget type (`ChatFrame1: PrimaryChatFrameMixin & … & Frame` has `ApplySystemAnchor`), and the mixins are declared with the methods their Lua defines (parameters `any`)
- W8040 is not reported inside an existence test of the hooked member (`if obj.Method then hook(obj, "Method", fn) end`); W8102 still is
- The standard library is safe with secret values throughout: no module compares, indexes, measures or truth-tests a value the client may hand back as a secret, or a table whose security keeps addons out. Values only carried pass through unchanged; where an operation needs the value, it is skipped or read as unknown, as each module's chapter now says. Among the paths that raised in game before:
  - `wowlua:table`: `merge` and `defaults` walked secret tables; `contains` compared secrets
  - `wowlua:saved`: profile copies and migrations walked secret tables
  - `wowlua:serialize`: `encode` refused any value holding a secret (now encoded as nil); `decode` and `fromPrintable` of a secret string
  - `wowlua:comm` and `wowlua:version`: a secret message, target, prefix or sender
  - `wowlua:errors`: an error whose `tostring` is secret ended in "error in error handling" and was never reported
  - `wowlua:aura`: aura tables addons may not index reached `find`; a secret spell ID in a container's `spells`
  - `wowlua:unit`, `wowlua:format`, `wowlua:menu`, `wowlua:dialog`, `wowlua:launcher`, `wowlua:alert`: secret booleans in options and answers
  - `wowlua:tooltip`, `wowlua:color`, `wowlua:links`, `wowlua:media`, `wowlua:pool`, `wowlua:list`, `wowlua:frames`, `wowlua:settings`, `wowlua:slash`, `wowlua:reflect`, `wowlua:debug`: secret links, colors, names, keys, indexes, positions and conditions
- `wowlua:test` and `wowlua:mock`: a matcher, spy, snapshot or wait given a secret fails with a message saying so instead of raising inside the framework; new matcher `toBeSecret`

## 0.9.1 (2026-10-20)

M9 so far, released before the milestone closes: wowlua's emulation of the WoW
client, and the test framework that runs on it. The language server, the VS
Code extension and the milestones after M9 are planned; see the
[roadmap](spec/roadmap.md).

### New

- `wowlua-vm`, a Lua 5.1 VM in Rust: the corpus compiles to `luac`'s bytecode and lines, and the Lua 5.1 test suite passes (S1)
- WoW's Lua environment in the VM: taint and secure execution with taint reports, secret values, the client's limits and call depths (S2)
- `wowlua-engine`: the WoW API from Blizzard's documentation with its argument checks, frames, events, timers, units, auras and protected actions; Blizzard's UI code loads as secure code, and on WoW: Forever it logs in and runs without an error, as the client's does (S3)
- `wowlua test`: a package's test files run per flavor in the emulated client, with the addon loaded from its TOC as the client loads it; `test.blizzard-ui` loads Blizzard's UI first, `test.addons` other addons' folders, each as its own addon; `--watch` ([docs](docs/getting-started.md#test-it), S4)
- `wowlua:test`: tests, hooks, `@skip`, `@only`, `@flavors`, `@cases`, `describe`, typed matchers, widget matchers, snapshots (`-u` updates them), custom matchers and compile-time tests with `@expectError` (S4)
- `wowlua:mock`: units, combat, time and events of a simulated world, spies, stubs and fakes, waiting, printed output and errors, `mock.module`, user input (click, hover, drag, typing, key bindings), casting, vendors, `mock.reloadUI`, and taint assertions (`toBeBlocked`, `toBeSecure`) (S4)
- World profiles: a test file starts with the character, money, gear, bags, place, auras and units of `tests/profiles/<name>.json`; `/stdtest snapshot` records one in the client (S4)
- `import internal` reaches a module's private declarations from tests (S4)
- Coverage and reports: `wowlua test --coverage` (lines, statements, branches, functions, conditions), `test.coverage` minimums, and the `junit`, `tap`, `json`, `github`, `lcov`, `cobertura`, `coverage-json` and `html` reporters (S5)
- The three examples have tests
- The standard library's tests run in the VM, on the engine
- `-j, --jobs` (`WOWLUA_JOBS`, `build.jobs`) and `build.jobs-memory`: the worker threads of a build and the memory they may take
- Lua compile results are kept between runs in the cache (`build.compile-cache`, `build.compile-cache-size`, `WOWLUA_NO_COMPILE_CACHE`)
- The emulated client answers WoW: Forever's whole native API and its widget methods; answers come from client recordings where there are any, from models of the game's state (bags, mail, quests, auras, the group finder…) and otherwise from Blizzard's documentation (S4)
- Twenty-four recorded client scenarios replay in the engine (bank, vendor, mail, auction, taxi, talents, professions, quests, combat, death, the group finder, repair, bags, the quest log…), checked line by line against the client (S4)
- SuuUI, Ellesmere UI and ElvUI run in `wowlua test` with Blizzard's UI as in the client, ElvUI with exactly the client's errors (S4)
- `/stdtest engine answers`, `/stdtest engine fullprobe` and `/stdtest engine methodprobe` (every widget method on fresh hidden widgets) ask the client the engine's open questions; `cargo xtask ingest` turns the answers into the engine's data (S4)

### Changed

- `wowlua fmt` formats all of wowlua's syntax (S6)
- Builds parse, lower, check, lint and walk the load order's call graph on worker threads
- Builds take less memory: smaller syntax trees and IR, each source's text kept once, source maps kept encoded
- `wowlua dev` rebuilds once per batch of changes, keeps compiled files between builds, does not parse unchanged plain Lua files again and writes only the files that changed
- Runs end with a count of errors and warnings; a source line longer than 160 characters is shown as a window around its labels
- Load order: no W1033 for files whose order `build.load-order` or an XML file fixes
- Load order: a load-on-demand addon may read its dependencies' tables while it loads without W1034
- Load order: branches that run only when an argument is not a function no longer count for callers that pass one (fewer W1035)

### Fixed

- Long operator chains build without a stack overflow or quadratic time, also on Windows
- `check.lints` accepts the warning forms of name and type errors (`"W2001": "allow"`)
- `wowlua dev` ignores the output folder `build.out` names, not only `dist/`
- Field assignments of an annotated variable are checked against its declared type

## 0.9.0 (2026-10-15)

M8, the standard library: `wowlua:` modules written in wowlua, typed, and
tested in game on WoW: Forever. `examples/core-demo` uses no third-party
libraries for its events, saved variables, settings, minimap button,
keybindings and comms.

### New

- Core: `events`, `lifecycle`, `timer`, `combat`, `hooks`, `slash`, `saved`, `callbacks`, `errors`, `jobs`, `table`, `format`
- Interface: `locale` (JSON translations, a typed `L`), `pool`, `settings`, `launcher`, `frames` (with Edit Mode), `bindings`, `menu`, `list`, `dialog`, `color`, `media`, `alert`, `links` and widget themes
- Game data, secret-value aware: `secret`, `unit`, `aura`, `spell`, `item`, `tooltip`; and `secure`, `serialize`, `comm`, `version`
- `wowlua compile`: files to Lua without an AddOn folder
- WoW's Lua environment as probed in game, with W2060 for a literal zero divisor

### Changed

- Builds are about 5x faster with 2–3x less memory on large multi-flavor addons
- The runtime and the mock client are written in wowlua

### Fixed

- Variable-file folders load only in their own flavor
- Load order: shared files are not loaded by the shipping addon, and API functions do not call their arguments
- Declared table types bind exactly in generic calls

## 0.8.2 (2026-10-13)

### Fixed

- Load order: fields of global and addon tables (`ns.Tools.Run`) order files too ([#1](https://github.com/DomenikIrrgang/wowlua/issues/1))

## 0.8.1 (2026-10-13)

M7's exit criterion is met: SuuUI, a WoW: Forever addon adopted with
`wowlua init`, was published from CI to CurseForge, Wago and GitHub with one
tag push.

### Changed

- CurseForge, Wago and WoWInterface targets build unminified unless the target sets `minify`
- W6010 warns when an addon-site target publishes minified code, no longer for GitHub targets

## 0.8.0 (2026-10-13)

M7, optimization and publishing: bundled, minified and optimized release
builds, and `wowlua publish` to four addon sites.

### New

- `wowlua package`: target builds and reproducible zips
- `wowlua publish` to CurseForge, Wago, WoWInterface and GitHub
- `wowlua login` / `logout` with the OS credential store
- Bundling with segments, bundle source maps, and `wowlua trace` through maps
- Minification passes and levels, with W6010 for renaming levels on publish targets
- Inline, cache-globals and dead-code optimizations
- `build --stats`, shared bundles and script-only XML flattening
- `publish.bundle-addons`; member targets stay with their project

### Changed

- The CI template publishes on tags; `init` shows how to switch packager workflows
- Optimizations never break Lua limits

### Fixed

- `publish` tags without a git identity
- Packager token variables are read from `.env` only

## 0.7.0 (2026-10-12)

M6, the package manager. Addons built with the BigWigs packager migrate with
`wowlua init --from-pkgmeta` and build the same, checked on thirty-six addons.

### New

- Git, SVN, path and archive sources, a global cache and `wowlua.lock`
- `add`, `remove`, `update`, `fetch`, `tree`, `outdated`, `cache clean`
- Workspaces for addon suites, with generated declaration files
- `.pkgmeta` import and packager compatibility mode
- Per-flavor load orders, SavedVariables and publish IDs; external addons

### Fixed

- Shipped files compare case-insensitively, like the game's file systems
- SVN trunk libraries on a tag take the tag's revision, like the packager
- Packager parity for TOC versions, line endings and shipped files

## 0.6.0 (2026-10-11)

M5, debugging: `wowlua:debug`, `wowlua:reflect` and `wowlua:log`; profiling,
tracing and runtime checks; the in-game dev window (`/wl`); `wowlua trace`.

### New

- `wowlua:debug`, `wowlua:reflect` and `wowlua:log`
- Profiling, tracing and runtime type checks
- Dev builds with every tool built in: every function profiled, events, scripts and timers monitored, errors captured
- The dev window (`/wl`) with profiler, monitor, log, errors, watch and inspector tabs, and `/wl` chat commands
- `wowlua trace` for line-preserved builds

## 0.5.0 (2026-10-11)

M4, the type checker: basic and strict mode, generics, narrowing, flavor
guards, `wowlua lint` and `wowlua fix`.

### New

- Basic mode: inference, narrowing, assignability, arity, globals, classes, interfaces, generics, overloads
- Strict mode and `Secret<T>`
- The WoW API of every flavor, generated from Blizzard's documentation
- Declaration files and `wowlua types convert` for LuaLS annotations
- `shared` flavor mode with flavor guards (E2030)
- `wowlua lint` and `wowlua fix`

## 0.4.0 (2026-10-11)

M3: classes, widget classes, enums and decorators, and the standard-library
infrastructure.

### New

- Classes (abstract and generic), widget classes, interfaces (parsed) and enums
- Core decorators (`@global`, `@deprecated`, `@if`, `@inline`) and user decorators
- Standard-library infrastructure: `wowlua:*` modules compiled into addons and tree-shaken
- `examples/widget-demo`

## 0.3.0 (2026-10-11)

M2: lowering of syntax sugar and modules.

### New

- `+=`, `continue`, `?.`, `??`, string interpolation, bitwise operators, `//`, `const`, default parameters and destructuring
- `import` / `export` modules
- Load-order analysis of globals and addon-table fields, per flavor
- `wowlua:meta`, static conditions, `@if`, flavor split builds and XML reference rewriting

## 0.2.0 (2026-10-11)

M1: the parser and the emitter.

### New

- A parser for Lua 5.1 and all of wowlua's syntax, with error recovery
- The Lua emitter, with line modes and source maps
- `.wowlua` files compile with their type annotations erased
- `wowlua check` (syntax only) and a first `wowlua fmt` for Lua 5.1

## 0.1.0 (2026-10-11)

M0, the foundation: an existing Lua addon builds and installs from a
`wowlua.json`, with generated TOCs for several flavors.

### New

- `new`, `init` (from a TOC), `build` for plain Lua projects (TOC generation, assets, `LICENSES.txt`, output validation), `install`, `uninstall`, `dev` (copy and link modes, safe installs), `clean`, `toc` and `explain`
- `.wowlua` files that are pure Lua 5.1
- `wowlua.lock` with the pinned flavor table
- Release binaries for macOS, Linux and Windows on GitHub Releases

