-- /stdtest engine scenario mail2, played with the engine's controls: the
-- level 21 paladin at the Crossroads' mailbox, its inbox not yet fetched
-- this session, opens it; reads a sold auction's letter and takes its
-- money; reads a won auction's letter and takes its Crafted Light Shot
-- (scenarios/mail2.lua). Each step at its recorded time (a click's
-- release), each answer's round trip as recorded.
local e = __engine
local player = e.player
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
-- The letters' items (the recording's answers).
e.defineItem(8067, { name = "Crafted Light Shot", quality = 1, icon = 132384, stackCount = 200, classID = 6, subclassID = 3, sellPrice = 0 })
e.defineItem(858, { name = "Lesser Healing Potion", quality = 1, icon = 134830, stackCount = 5, classID = 0, subclassID = 1 })
local mailbox = e.spawnUnit({ name = "Mailbox", guid = "GameObject-0-4621-1-21-143981-0000000001", reaction = "friendly", level = 1 })
-- The inbox as the server sent it: its auction letters' subjects and
-- invoices, their items' names "Retrieving data" till the Light Shot's data
-- came (0.011 s after the inbox), the invoices' buyers named 0.497 s after
-- the inbox; a letter read is told at once (none waited on the server); an
-- item taken pushed into the bags 0.147 s after the take, its data 1.062 s
-- after that; an emptied letter gone 0.37 s after the take.
e.defineMailbox(mailbox, {
    newMail = true,
    senders = { "Horde Auction House" },
    fetched = false, dataAfter = 0.011, received = { 8067 }, namesAfter = 0.497,
    bagAfter = 0.147, goneAfter = 0.37,
    letters = {
        {
            stationeryIcon = 134939, sender = "Horde Auction House", subject = "Auction successful: Battered Leather Gloves",
            money = 195, daysLeft = 28.488321304321, textCreated = true, retrieving = true, readAfter = 0, auction = true,
            invoice = { "seller", "Battered Leather Gloves", "Otherplayer", 100, 100, 100, 5, 0, 0, 0, 1, false },
        },
        {
            stationeryIcon = 134939, sender = "Horde Auction House", subject = "Auction won: Crafted Light Shot",
            daysLeft = 28.43404006958, textCreated = true, retrieving = true, readAfter = 0, auction = true,
            items = { { id = 8067, count = 1, itemAfter = 1.062 } },
            invoice = { "buyer", "Crafted Light Shot", "Otherplayer", 1, 1, 0, 0, 0, 0, 0, 1, false },
        },
        {
            stationeryIcon = 134939, sender = "Horde Auction House", subject = "Auction won: Crafted Light Shot",
            daysLeft = 27.645566940308, textCreated = true, retrieving = true, readAfter = 0, auction = true,
            items = { { id = 8067, count = 1 } },
            invoice = { "buyer", "Crafted Light Shot", "Otherplayer", 1, 1, 0, 0, 0, 0, 0, 1, false },
        },
        {
            stationeryIcon = 134939, sender = "Horde Auction House", subject = "Auction expired: Lesser Healing Potion (3)",
            daysLeft = 1.3168865442276, wasRead = true, textCreated = true, items = { { id = 858, count = 3 } }, auction = true,
        },
    },
})
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
-- The world's own, not modelled: the server's events as recorded.
local function world(...) e.fireEvent(...) end
-- A spell (1229739) the client tries now and then, failing quietly.
local function tried(guid)
    world("UNIT_SPELLCAST_FAILED_QUIET", "player", "Cast-3-4621-1-21-1229739-" .. guid, 1229739, nil)
    world("CURRENT_SPELL_CAST_CHANGED", true)
    world("ACTIONBAR_UPDATE_STATE")
end
-- The mailbox opened at the right-click's release (0.137 s after its
-- press); the inbox 0.511 s after Blizzard's mail frame asks for it.
mailbox:setDistance(2)
e.setLatency(0.137)
at(1.260) player:interact(mailbox)
e.setLatency(0.511)
at(1.6)
check("the mailbox open", MailFrame:IsShown())
at(2.6)
check("four letters", InboxFrame:IsShown() and GetInboxNumItems() == 4)
-- The sold gloves' letter read, its money taken (0.407 s).
at(4.545) press(_G["MailItem1Button"])
at(4.8)
check("the letter open", OpenMailFrame:IsShown())
e.setLatency(0.407)
at(7.657) press(OpenMailMoneyButton)
at(9.0)
check("the money taken, the letter gone", GetInboxNumItems() == 3 and GetMoney() == 19116)
at(9.097) tried("9C919B4229")
-- The won Light Shot's letter read, the shot taken (0.348 s).
at(12.380) press(_G["MailItem1Button"])
at(14.013) tried("3881CB0F9A")
e.setLatency(0.348)
at(14.729) press(OpenMailFrame.OpenMailAttachments[1])
-- Another player's death nearby.
at(16.286) world("UNIT_DIED", "Player-0000-00000000")
at(17)
check("the shot taken, the letter gone", GetInboxNumItems() == 2 and C_Container.GetContainerItemID(0, 17) == 8067)
at(19.024) tried("C8F635ACA8")
at(23.4)
