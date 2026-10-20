-- /stdtest engine scenario trainer, played with the engine's controls: the
-- level 2 warrior talks to Llane Beshere (Northshire's warrior trainer),
-- asks for training, learns Battle Shout (10 copper) and closes the window
-- (scenarios/trainer.lua). Each step at its recorded time (a click: a
-- moment before the server's answer), the ping as recorded (~0.3 s).
local e = __engine
local player = e.player
e.setLatency(0.3)
-- The warrior's names as the recording has them, scrubbed.
player:setName("Player", "Lastname", { quiet = true })
local llane = e.spawnUnit({
    name = "Llane Beshere", guid = "Creature-0-6782-0-7-911-FD7AF506B9", npcID = 911, level = 5,
    reaction = "friendly", class = "WARRIOR", classID = 1, sex = 2, powerType = 1,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Alliance",
})
e.defineGossip(llane, {
    -- The text's packet 0.472 s after the interaction (recorded).
    textAfter = 0.472,
    text = "Stormwind is the pillar of the human race, maintained through strength of will and the edge of a sturdy blade.",
    options = { { name = "I require warrior training.", icon = 132058, gossipOptionID = 96654, opens = "trainer" } },
})
-- Its services as the client listed them (Northshire's: the recorded
-- WARRIOR-911); Rend's rank still loading that session, its greeting empty.
e.defineTrainer(llane, { class = "WARRIOR", greeting = "", loading = { [772] = 9118 } })
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
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
local function chatHas(frame, text)
    for i = 1, frame:GetNumMessages() do
        if (frame:GetMessageInfo(i)) == text then return true end
    end
    return false
end
-- A shown button (in a scroll box's rows) by its text.
local function button(root, text, depth)
    depth = depth or 0
    if depth > 8 or not root or root:IsForbidden() then return nil end
    if root:IsObjectType("Button") and root:IsVisible() then
        local t = root.GetText and root:GetText()
        if t == text then return root end
        for _, r in ipairs({ root:GetRegions() }) do
            if r:IsObjectType("FontString") and r:GetText() == text then return root end
        end
    end
    for _, c in ipairs({ root:GetChildren() }) do
        local found = button(c, text, depth + 1)
        if found then return found end
    end
end
at(0.819) player:target(llane)
llane:setDistance(3)
player:interact(llane)
at(2.0)
check("the gossip: Llane's text", GossipFrame:IsShown())
local option = button(GossipFrame, "I require warrior training.")
check("the gossip's option shown", option ~= nil)
at(2.447) if option then press(option) end
at(3.0)
check("the trainer's window", ClassTrainerFrame and ClassTrainerFrame:IsShown())
local row = ClassTrainerFrame and button(ClassTrainerFrame, "Battle Shout")
check("Battle Shout listed", row ~= nil)
at(3.744) if row then press(row) end
-- Another player's Blessing of Might on the warrior, its attack power a
-- moment later (the server's, recorded).
at(4.066) player:applyAura({
    spellId = 19834, name = "Blessing of Might", icon = 135906, duration = 3600,
    dispelName = "Magic", auraInstanceID = 65,
})
at(4.370) if ClassTrainerTrainButton then press(ClassTrainerTrainButton) end
at(4.377)
e.fireEvent("UNIT_DAMAGE", "player")
e.fireEvent("UNIT_DAMAGE", "player")
e.fireEvent("UNIT_ATTACK_POWER", "player")
at(5.0)
check("chat: Battle Shout learned", chatHas(ChatFrame1, "You have learned a new ability: |cff71d5ff|Hspell:6673:0|h[Battle Shout]|h|r."))
check("Battle Shout on the bar's third slot", select(2, GetActionInfo(3)) == 6673)
at(5.653) if ClassTrainerFrame then HideUIPanel(ClassTrainerFrame) end
check("the trainer's window closed", ClassTrainerFrame and not ClassTrainerFrame:IsShown())
at(5.869) player:target(nil)
at(6.5)
check("the money: 24 copper", GetMoney() == 24)
