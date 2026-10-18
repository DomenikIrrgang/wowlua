-- /stdtest engine scenario legacy, played with the engine's controls: the
-- level 21 druid opens the spellbook, then the legacy system, picks a
-- tree, buys five ranks of a node and applies them
-- (scenarios/legacy.lua). Each step at its recorded time. Not yet: the
-- spellbook's TOOLTIP_DATA_UPDATE(939) (6.916, its cause unknown); the
-- order of the nodes the fifth rank unlocks (the client's 110303, 110305,
-- 110304; the engine's the tree's).
local e = __engine
-- The druid's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 9885, 9910, 21850, 9756, 467, 8907, 1126, 21849, 2893, 9884, 1075, 5234, 5232, 8914, 5287, 782 })
-- Before: the legacy tree's node 110301 not bought yet (the probe's
-- ranks are the end's: 5 of 5); the legacy track's renown (the account's
-- answer, the tree's 16 points).
e.setTraitRanks(22158, { [110301] = 0 })
e.setRenown(2802, 16)
-- The reward track's items, cached in the client (no load recorded): their
-- names and icons as its rewards name them (the air rifle's from its
-- toast); unverified: everything else of them.
for id, item in pairs({
    [276236] = { name = "Replica Ironforge Air Rifle", icon = 135614 },
    [277714] = { name = "Spectral Bear Cub", icon = 294471 },
    [277717] = { name = "Spectral Bear Tabard", icon = 7941423 },
    [277718] = { name = "Reins of the Spectral Bear", icon = 3753812 },
}) do
    e.defineItem(id, item, { cached = true })
end
local now = 0
-- Time passes in frames, as in the client (60 a second: Blizzard's
-- animations and OnUpdate code run), the rest of a step as one jump.
local function at(t)
    if t < now then error(("at(%s): before %s"):format(t, now), 2) end
    local frames = math.floor((t - now) * 60)
    if frames > 0 then
        e.advanceFrames(frames, 1000 / 60)
    end
    e.advanceTime(math.max(0, t - now - frames / 60))
    now = t
end
local function press(frame, button)
    if not frame then
        check("clicked: no frame", false)
        return
    end
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
e.setLatency(0.35)
at(4.812) press(PlayerSpellsMicroButton)
at(10.210) press(LegacyMicroButton)
at(12.455) press(LegacySystemFrame and LegacySystemFrame.LegacyChallengeTab)
at(13.235) press(LegacySystemFrame and LegacySystemFrame.LegacyTreeTab)
check("the legacy frame shown", LegacySystemFrame and LegacySystemFrame:IsShown())
local page = LegacySystemFrame.TreePage
check("its tree page shown", page:IsShown())
-- Two clicks on the frame itself, then the second tree (adventure: 1188).
at(16.239) press(LegacySystemFrame)
at(16.476) press(LegacySystemFrame)
at(17.017) press(page.LegacyTreeSelectionPanel.treeButtons[2])
local panel = page.LegacyTreeTraitPanel
local node = panel:GetTalentButtonByNodeID(110301)
check("the node's button shown", node and node:IsVisible())
-- Five ranks bought, a sixth click at the last.
for _, t in ipairs({ 20.158, 23.681, 23.861, 24.053, 24.234, 24.383 }) do
    at(t) press(node)
end
at(25.186) press(LegacySystemFrame)
at(25.401) press(LegacySystemFrame)
at(25.598) press(panel.ApplyButton)
at(27.0)
check("the ranks applied", C_Traits.GetNodeInfo(22158, 110301).ranksPurchased == 5
    and not C_Traits.ConfigHasStagedChanges(22158))
at(28.4)
