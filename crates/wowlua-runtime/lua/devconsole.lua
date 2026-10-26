-- wowlua runtime: the dev console (spec §7.7): the dev window (§7.7.1), the
-- HUD (§7.7.8), the error toast (§7.10) and the `/wl` commands, drawn and
-- served by the newest console loaded. The look follows the wowlua design
-- system (decision T171): dark flat surfaces, 1px lines, amber for what is
-- chosen, the signal colours only for what they mean.

-- The kit: colours, fonts and widgets the window, the HUD and the toast
-- share.
local K = {}
do
	local function hex(s, a)
		return {
		(tonumber(s:sub(1, 2), 16) or 0) / 255,
		(tonumber(s:sub(3, 4), 16) or 0) / 255,
		(tonumber(s:sub(5, 6), 16) or 0) / 255,
		a or 1
		}
	end
	-- The design system's game tokens.
	K.C = {
	glass = hex("14120e", 0.94),
	surface = hex("1a1813"),
	sunk = hex("0f0d0a"),
	raised = hex("24201a"),
	selected = hex("2e2416"),
	line = hex("3a342a"),
	lineStrong = hex("6e6554"),
	ink = hex("ece6d6"),
	muted = hex("a39b8a"),
	faint = hex("756d5f"),
	accent = hex("ffb547"),
	onAccent = hex("1a1308"),
	error = hex("ff7262"),
	warning = hex("ecd660"),
	note = hex("74c7ec"),
	success = hex("6fdcb4"),
	accentSoft = hex("ffb547", 0.35),
	noteSoft = hex("74c7ec", 0.35),
	errorSoft = hex("ff7262", 0.45),
	successSoft = hex("6fdcb4", 0.45)
	}
	local DIR = "Interface\\AddOns\\" .. addonName .. "\\wowlua\\fonts\\"
	K.FONTS = {
	display = DIR .. "MartianMono-StdBd.ttf",
	body = DIR .. "IBMPlexMono-Regular.ttf",
	label = DIR .. "IBMPlexMono-Medium.ttf",
	numeric = DIR .. "IBMPlexMono-SemiBold.ttf"
	}
	do local __wl_t1 = STANDARD_TEXT_FONT if __wl_t1 == nil then __wl_t1 = "Fonts\\FRIZQT__.TTF" end K.FALLBACK = __wl_t1 end
	-- A widget's measure or text, or `fallback` while it reads secret (or
	-- nil): the window lays itself out from plain values.
	function K.plain(v, fallback)
		if type(v) ~= "number" or (issecretvalue and issecretvalue(v)) then
			return fallback
		end
		return v
	end
	function K.plainText(v)
		if type(v) ~= "string" or (issecretvalue and issecretvalue(v)) then
			return ""
		end
		return v
	end
	-- Whether a widget is shown (a secret answer counts as hidden).
	function K.isShown(f)
		local v = f:IsShown()
		return v == true
	end
	function K.font(fs, kind, size)
		if not fs:SetFont(K.FONTS[kind] or K.FONTS.body, size, "") then
			fs:SetFont(K.FALLBACK, size, "")
		end
	end
	function K.text(parent, kind, size, color)
		local fs = parent:CreateFontString(nil, "OVERLAY")
		K.font(fs, kind, size)
		fs:SetTextColor(unpack(color or K.C.ink))
		fs:SetJustifyH("LEFT")
		fs:SetWordWrap(false)
		return fs
	end
	function K.fill(parent, color, layer)
		local t = parent:CreateTexture(nil, layer or "BACKGROUND")
		t:SetColorTexture(unpack(color))
		return t
	end
	-- A 1px border inside `frame`.
	function K.border(frame, color)
		local edges = {}
		for i = 1, 4 do
			edges[i] = K.fill(frame, color, "BORDER")
		end
		edges[1]:SetPoint("TOPLEFT")
		edges[1]:SetPoint("TOPRIGHT")
		edges[1]:SetHeight(1)
		edges[2]:SetPoint("BOTTOMLEFT")
		edges[2]:SetPoint("BOTTOMRIGHT")
		edges[2]:SetHeight(1)
		edges[3]:SetPoint("TOPLEFT")
		edges[3]:SetPoint("BOTTOMLEFT")
		edges[3]:SetWidth(1)
		edges[4]:SetPoint("TOPRIGHT")
		edges[4]:SetPoint("BOTTOMRIGHT")
		edges[4]:SetWidth(1)
		return edges
	end
	function K.recolor(edges, color)
		for _, e in ipairs(edges) do
			e:SetColorTexture(unpack(color))
		end
	end
	function K.hairline(parent, color)
		local t = K.fill(parent, color or K.C.line, "ARTWORK")
		t:SetHeight(1)
		return t
	end
	-- A quiet button, or `primary` (the one action of a view, in amber).
	function K.button(parent, label, onClick, primary)
		local C = K.C
		local b = CreateFrame("Button", nil, parent)
		b:SetHeight(26)
		local bg = K.fill(b, primary and C.accent or C.raised)
		bg:SetAllPoints()
		local edges = K.border(b, primary and C.accent or C.line)
		local fs = K.text(b, "label", 12, primary and C.onAccent or C.ink)
		fs:SetPoint("CENTER")
		b.label, b.bg, b.edges, b.primary = fs, bg, edges, primary
		function b:SetLabel(s)
			fs:SetText(s)
			self:SetWidth(math.max(56, K.plain(fs:GetStringWidth(), 40) + 22))
		end
		b:SetLabel(label)
		b:SetScript("OnEnter", function()
			if not b.primary then
				K.recolor(edges, C.lineStrong)
			end
		end)
		b:SetScript("OnLeave", function()
			K.recolor(edges, b.primary and C.accent or C.line)
		end)
		b:SetScript("OnClick", onClick)
		return b
	end
	-- A switch: a button that shows whether it is on.
	function K.toggle(parent, label, onClick)
		local C = K.C
		local b = K.button(parent, label, onClick)
		function b:SetOn(on)
			self.bg:SetColorTexture(unpack(on and C.selected or C.raised))
			K.recolor(self.edges, on and C.accent or C.line)
			self.label:SetTextColor(unpack(on and C.accent or C.muted))
			self.label:SetText((on and "✓ " or "") .. label)
			self:SetWidth(math.max(56, K.plain(self.label:GetStringWidth(), 40) + 22))
		end
		b:SetOn(false)
		return b
	end
	-- A field on the sunk surface.
	function K.field(parent, width, placeholder, onEnter)
		local C = K.C
		local box = CreateFrame("EditBox", nil, parent)
		box:SetSize(width, 26)
		box:SetAutoFocus(false)
		local bg = K.fill(box, C.sunk)
		bg:SetAllPoints()
		K.border(box, C.line)
		K.font(box, "body", 12)
		box:SetTextColor(unpack(C.ink))
		box:SetTextInsets(8, 8, 0, 0)
		local hint = K.text(box, "body", 12, C.faint)
		hint:SetPoint("LEFT", 8, 0)
		hint:SetText(placeholder)
		box:SetScript("OnTextChanged", function(self)
			hint:SetShown(K.plainText(self:GetText()) == "")
			if self.onChange then
				self.onChange(K.plainText(self:GetText()))
			end
		end)
		box:SetScript("OnEscapePressed", box.ClearFocus)
		box:SetScript("OnEnterPressed", function(self)
			if onEnter then
				onEnter(K.plainText(self:GetText()))
			end
			self:ClearFocus()
		end)
		return box
	end
	-- Two to eight named options; the chosen one in amber.
	function K.segmented(parent, options, onPick)
		local C = K.C
		local f = CreateFrame("Frame", nil, parent)
		f:SetHeight(26)
		local bg = K.fill(f, C.sunk)
		bg:SetAllPoints()
		K.border(f, C.line)
		local x = 1
		f.buttons = {}
		for _, opt in ipairs(options) do
			local b = CreateFrame("Button", nil, f)
			local fs = K.text(b, "label", 11, C.muted)
			fs:SetPoint("CENTER")
			fs:SetText(opt.label)
			local w = math.max(44, K.plain(fs:GetStringWidth(), 40) + 16)
			b:SetSize(w, 24)
			b:SetPoint("TOPLEFT", x, -1)
			x = x + w
			local on = K.fill(b, C.selected)
			on:SetAllPoints()
			b.on, b.fs, b.value = on, fs, opt.value
			b:SetScript("OnClick", function()
				f:Select(opt.value)
				onPick(opt.value)
			end)
			f.buttons[#f.buttons + 1] = b
		end
		f:SetWidth(x + 1)
		function f:Select(value)
			for _, b in ipairs(self.buttons) do
				local chosen = b.value == value
				b.on:SetShown(chosen)
				b.fs:SetTextColor(unpack(chosen and C.accent or C.muted))
			end
		end
		return f
	end

	-- Numbers as the window shows them.
	function K.ms(v)
		local n = v or 0
		return string.format(n >= 100 and "%.0f" or (n >= 10 and "%.1f" or "%.2f"), n)
	end
	function K.count(n)
		local s = tostring(math.floor((n or 0) + 0.5))
		local out = s:reverse():gsub("(%d%d%d)", "%1,"):reverse()
		return (out:gsub("^,", ""):gsub("^%-,", "-"))
	end
	function K.kb(v)
		local n = v or 0
		if n >= 1024 then
			return string.format("%.1f MB", n / 1024)
		end
		return string.format(n >= 10 and "%.0f KB" or "%.1f KB", n)
	end
	function K.duration(s)
		local n = s or 0
		if n >= 3600 then
			return string.format("%dh %02dm", math.floor(n / 3600), math.floor(n % 3600 / 60))
		elseif n >= 60 then
			return string.format("%dm %02ds", math.floor(n / 60), math.floor(n % 60))
		end
		return string.format("%ds", math.floor(n))
	end
	-- A figure per minute of a recording (decision T173).
	function K.perMinute(v, data)
		do local __wl_t2 = data.duration if __wl_t2 == nil then __wl_t2 = 0 end return (v or 0) / math.max((__wl_t2) / 60, 1 / 60) end
	end
	-- The change of `a` against `b` (both per minute): text and colour;
	-- within ±1% unchanged. `lowerIsBetter` colours a rise as worse.
	function K.change(a, b, lowerIsBetter)
		local C = K.C
		local x, y = a or 0, b or 0
		if y == 0 and x == 0 then
			return "=", C.faint
		elseif y == 0 then
			return "new", C.warning
		elseif x == 0 then
			return "gone", C.success
		end
		local r = (x - y) / y
		if math.abs(r) <= 0.01 then
			return "=", C.faint
		end
		local worse = (r > 0) == (lowerIsBetter ~= false)
		local s = r > 0 and string.format("↑ %d%%", math.floor(r * 100 + 0.5)) or string.format("↓ %d%%", math.floor(-r * 100 + 0.5))
		return s, worse and C.error or C.success
	end
end

-- The saved data of an addon, `WowluaDevDB_<Addon>` (decision T103): the
-- window's state in `window`, the HUD's in `hud`, the addon's switches in
-- `settings` and its exported recordings in `exports`.
local function devDB(name)
	local key = "WowluaDevDB_" .. name:gsub("[^%w_]", "_")
	local db = _G[key]
	if type(db) ~= "table" then
		db = {}
		_G[key] = db
	end
	-- 0.12.0 saved the window's state at the top.
	if db.point and not db.window then
		db.window = {point = db.point, x = db.x, y = db.y, width = db.width, height = db.height, tab = db.tab, addon = db.addon}
		db.point, db.x, db.y, db.width, db.height, db.tab, db.addon = nil, nil, nil, nil, nil, nil, nil
	end
	return db
end

-- Performance hints (§7.7.7): the rules over `live`, dismissed hints
-- hidden until they get worse (by half).
local function runHints(d)
	local perf = d.runtime.perf
	local h = d.hintState
	if not h then
		h = {dismissed = {}, seen = {}, list = {}}
		d.hintState = h
	end
	if not perf then
		h.list = {}
		return h.list
	end
	local shown = {}
	for _, x in ipairs(perf.hints(perf.data(perf.recordings[1]))) do
		local id = x.code .. " " .. x.subject
		x.id = id
		local was = h.dismissed[id]
		if was and x.metric > was * 1.5 then
			h.dismissed[id], h.seen[id] = nil, nil
			was = nil
		end
		if not was then
			shown[#shown + 1] = x
		end
	end
	h.list = shown
	return shown
end
local function newHints(d)
	local h = d.hintState
	local n = 0
	for _, x in ipairs(h and h.list or {}) do
		if not h.seen[x.id] then
			n = n + 1
		end
	end
	return n
end

local function buildDevWindow(console, reg)
	local C, plain = K.C, K.plain




	local TABS = {"Overview", "Performance", "Monitor", "Log", "Errors", "Watch", "Inspector", "Hints"}
	local VIEW_KEYS = {"overview", "performance", "monitor", "log", "errors", "watch", "inspector", "hints"}
	local PERF_VIEWS = {
	{value = "functions", label = "Functions"},
	{value = "tree", label = "Tree"},
	{value = "flame", label = "Flame"},
	{value = "events", label = "By event"},
	{value = "timeline", label = "Timeline"},
	{value = "slow", label = "Slow"},
	{value = "garbage", label = "Garbage"},
	{value = "bench", label = "Bench"}
	}

	-- The window.
	local win = CreateFrame("Frame", "WowluaDevWindow", UIParent)
	win:SetSize(880, 640)
	win:SetPoint("CENTER")
	win:SetFrameStrata("DIALOG")
	win:SetClampedToScreen(true)
	win:SetMovable(true)
	win:SetResizable(true)
	win:EnableMouse(true)
	win:Hide()
	if win.SetResizeBounds then
		win:SetResizeBounds(760, 520)
	end
	local ground = K.fill(win, C.glass)
	ground:SetAllPoints()
	K.border(win, C.lineStrong)
	if UISpecialFrames then
		table.insert(UISpecialFrames, "WowluaDevWindow")
	end

	-- What the window shows; position, size, tab and addon are saved.
	local S = {
	tab = 2, sort = "total", monitor = "events", level = "all", filter = "", expanded = {},
	view = "functions", recOf = {}, compareOf = {}, treeOpen = {}, recOffset = 0,
	tlOffset = 0, benchN = 10000, benchExpr = ""
	}
	-- The window's parts, by name (one table, so its functions stay small).
	local U = {win = win}

	-- Title bar: the title, the key chip, the addon selector, close.
	local titleBar = CreateFrame("Frame", nil, win)
	titleBar:SetPoint("TOPLEFT")
	titleBar:SetPoint("TOPRIGHT")
	titleBar:SetHeight(40)
	titleBar:EnableMouse(true)
	titleBar:RegisterForDrag("LeftButton")
	titleBar:SetScript("OnDragStart", function()
		win:StartMoving()
	end)
	titleBar:SetScript("OnDragStop", function()
		win:StopMovingOrSizing()
		console.saveState()
	end)
	local mark = K.fill(titleBar, C.accent, "ARTWORK")
	mark:SetSize(3, 14)
	mark:SetPoint("LEFT", 14, 0)
	local title = K.text(titleBar, "display", 12, C.ink)
	title:SetPoint("LEFT", 24, 0)
	title:SetText("wowlua dev")
	local chip = K.text(titleBar, "numeric", 10, C.faint)
	chip:SetPoint("LEFT", title, "RIGHT", 10, 0)
	chip:SetText("/wl")
	local close = CreateFrame("Button", nil, titleBar)
	close:SetSize(24, 24)
	close:SetPoint("RIGHT", -8, 0)
	local closeMark = K.text(close, "label", 14, C.muted)
	closeMark:SetPoint("CENTER")
	closeMark:SetText("×")
	close:SetScript("OnClick", function()
		win:Hide()
	end)
	local titleRule = K.hairline(titleBar, C.line)
	titleRule:SetPoint("BOTTOMLEFT")
	titleRule:SetPoint("BOTTOMRIGHT")

	-- Addon selector: a dropdown field with the loaded dev builds.
	local addonButton = CreateFrame("Button", nil, titleBar)
	addonButton:SetSize(200, 26)
	addonButton:SetPoint("RIGHT", close, "LEFT", -10, 0)
	local addonBg = K.fill(addonButton, C.sunk)
	addonBg:SetAllPoints()
	local addonEdges = K.border(addonButton, C.line)
	local addonLabel = K.text(addonButton, "label", 12, C.ink)
	addonLabel:SetPoint("LEFT", 10, 0)
	addonLabel:SetPoint("RIGHT", -26, 0)
	local chevron = K.text(addonButton, "label", 12, C.muted)
	chevron:SetPoint("RIGHT", -10, 0)
	chevron:SetText("↓")
	local selected = reg.order[1]
	local function current()
		return reg.addons[selected] or reg.addons[reg.order[1]]
	end
	U.current = current
	-- The open list, directly under the field.
	local addonList = CreateFrame("Frame", nil, win)
	addonList:SetFrameStrata("FULLSCREEN_DIALOG")
	addonList:SetPoint("TOPRIGHT", addonButton, "BOTTOMRIGHT", 0, -2)
	addonList:Hide()
	local listBg = K.fill(addonList, C.surface)
	listBg:SetAllPoints()
	K.border(addonList, C.line)
	local addonRows = {}
	local function openAddonList()
		local y = -4
		for i, name in ipairs(reg.order) do
			local b = addonRows[i]
			if not b then
				b = CreateFrame("Button", nil, addonList)
				b:SetHeight(24)
				b.hover = K.fill(b, C.raised)
				b.hover:SetAllPoints()
				b.hover:Hide()
				b.mark = K.fill(b, C.accent, "OVERLAY")
				b.mark:SetSize(2, 14)
				b.mark:SetPoint("LEFT", 6, 0)
				b.fs = K.text(b, "label", 12, C.ink)
				b.fs:SetPoint("LEFT", 16, 0)
				b.info = K.text(b, "body", 11, C.muted)
				b.info:SetPoint("RIGHT", -10, 0)
				b.info:SetJustifyH("RIGHT")
				b:SetScript("OnEnter", function(self)
					self.hover:Show()
				end)
				b:SetScript("OnLeave", function(self)
					self.hover:Hide()
				end)
				addonRows[i] = b
			end
			local chosen = name == selected
			b.fs:SetText(name)
			b.fs:SetTextColor(unpack(chosen and C.ink or C.muted))
			b.mark:SetShown(chosen)
			local entry = reg.addons[name]
			do local __wl_t3 = entry if __wl_t3 ~= nil then __wl_t3 = __wl_t3.info.version end local __wl_t4 = __wl_t3 if __wl_t4 == nil then __wl_t4 = "" end b.info:SetText(__wl_t4) end
			b:SetScript("OnClick", function()
				selected = name
				addonList:Hide()
				console.saveState()
				console.refresh(true)
			end)
			b:ClearAllPoints()
			b:SetPoint("TOPLEFT", 1, y)
			b:SetPoint("TOPRIGHT", -1, y)
			b:Show()
			y = y - 24
		end
		for i = #reg.order + 1, #addonRows do
			addonRows[i]:Hide()
		end
		addonList:SetSize(math.max(plain(addonButton:GetWidth(), 200), 240), -y + 4)
		addonList:Show()
	end
	addonButton:SetScript("OnClick", function()
		if K.isShown(addonList) then
			addonList:Hide()
		else
			openAddonList()
		end
	end)
	addonButton:SetScript("OnEnter", function()
		K.recolor(addonEdges, C.lineStrong)
	end)
	addonButton:SetScript("OnLeave", function()
		K.recolor(addonEdges, C.line)
	end)

	-- Tabs; the hints tab shows the count of new hints.
	local tabBar = CreateFrame("Frame", nil, win)
	tabBar:SetPoint("TOPLEFT", 14, -40)
	tabBar:SetPoint("TOPRIGHT", -14, -40)
	tabBar:SetHeight(28)
	local tabRule = K.hairline(tabBar, C.line)
	tabRule:SetPoint("BOTTOMLEFT")
	tabRule:SetPoint("BOTTOMRIGHT")
	local tabButtons = {}
	for i, name in ipairs(TABS) do
		local b = CreateFrame("Button", nil, tabBar)
		local fs = K.text(b, "label", 12, C.muted)
		fs:SetPoint("LEFT")
		fs:SetText(name)
		b:SetHeight(28)
		local line = K.fill(b, C.accent, "ARTWORK")
		line:SetHeight(2)
		line:SetPoint("BOTTOMLEFT")
		line:SetPoint("BOTTOMRIGHT")
		local badge = CreateFrame("Frame", nil, b)
		badge:SetSize(18, 14)
		badge:SetPoint("LEFT", fs, "RIGHT", 4, 0)
		badge.bg = K.fill(badge, C.accent)
		badge.bg:SetAllPoints()
		badge.fs = K.text(badge, "numeric", 10, C.onAccent)
		badge.fs:SetPoint("CENTER")
		badge:Hide()
		b.fs, b.line, b.badge = fs, line, badge
		b:SetScript("OnClick", function()
			S.tab = i
			console.saveState()
			console.refresh(true)
		end)
		tabButtons[i] = b
	end
	local function layoutTabs()
		local tx = 0
		for _, b in ipairs(tabButtons) do
			local w = plain(b.fs:GetStringWidth(), 60) + 4
			if K.isShown(b.badge) then
				w = w + (plain(b.badge:GetWidth(), 18) + 4)
			end
			b:SetWidth(w)
			b:ClearAllPoints()
			b:SetPoint("LEFT", tx, 0)
			tx = tx + (w + 16)
		end
	end

	-- Body: a toolbar row (a second one for the performance tab), a canvas
	-- for charts, then a scrolling list.
	local body = CreateFrame("Frame", nil, win)
	body:SetPoint("TOPLEFT", 14, -78)
	body:SetPoint("BOTTOMRIGHT", -14, 46)
	local toolbar = CreateFrame("Frame", nil, body)
	toolbar:SetHeight(28)
	local toolbar2 = CreateFrame("Frame", nil, body)
	toolbar2:SetHeight(28)
	local canvas = CreateFrame("Button", nil, body)
	canvas:RegisterForClicks("LeftButtonUp", "RightButtonUp")
	canvas:EnableMouseWheel(true)
	local canvasBg = K.fill(canvas, C.sunk)
	canvasBg:SetAllPoints()
	K.border(canvas, C.line)
	local scroll = CreateFrame("ScrollFrame", nil, body)
	local list = CreateFrame("Frame", nil, scroll)
	list:SetSize(880 - 40, 10)
	scroll:SetScrollChild(list)
	scroll:EnableMouseWheel(true)
	local thumb = K.fill(body, C.lineStrong, "OVERLAY")
	thumb:SetWidth(3)
	local function layoutThumb()
		local view, total = plain(scroll:GetHeight(), 1), plain(list:GetHeight(), 1)
		if total <= view + 1 then
			thumb:Hide()
			return
		end
		thumb:Show()
		local h = math.max(20, view * view / total)
		local offset = plain(scroll:GetVerticalScroll(), 0) / (total - view) * (view - h)
		thumb:ClearAllPoints()
		thumb:SetPoint("TOPRIGHT", scroll, "TOPRIGHT", 6, -offset)
		thumb:SetHeight(h)
	end
	scroll:SetScript("OnMouseWheel", function(self, delta)
		local max = math.max(0, plain(list:GetHeight(), 0) - plain(self:GetHeight(), 0))
		self:SetVerticalScroll(math.min(max, math.max(0, plain(self:GetVerticalScroll(), 0) - delta * 40)))
		layoutThumb()
	end)
	-- The recordings, left of the performance views.
	local recPanel = CreateFrame("Frame", nil, body)
	recPanel:SetPoint("TOPLEFT")
	recPanel:SetPoint("BOTTOMLEFT")
	recPanel:SetWidth(190)
	local recBg = K.fill(recPanel, C.surface)
	recBg:SetAllPoints()
	K.border(recPanel, C.line)
	recPanel:EnableMouseWheel(true)
	U.recPanel = recPanel

	-- Lays the body out: `left` leaves room for the recordings, `bars` is
	-- the number of toolbars, `chart` the canvas's height.
	local function layout(left, bars, chart)
		local lx = left and 202 or 0
		recPanel:SetShown(left)
		toolbar:ClearAllPoints()
		toolbar:SetPoint("TOPLEFT", lx, 0)
		toolbar:SetPoint("TOPRIGHT")
		toolbar2:ClearAllPoints()
		toolbar2:SetPoint("TOPLEFT", lx, -34)
		toolbar2:SetPoint("TOPRIGHT")
		toolbar2:SetShown(bars > 1)
		local top = -(bars * 34 + 2)
		canvas:ClearAllPoints()
		canvas:SetPoint("TOPLEFT", lx, top)
		canvas:SetPoint("TOPRIGHT", -6, top)
		canvas:SetHeight(math.max(chart, 1))
		canvas:SetShown(chart > 0)
		if chart > 0 then
			top = top - (chart + 8)
		end
		scroll:ClearAllPoints()
		scroll:SetPoint("TOPLEFT", lx, top)
		scroll:SetPoint("BOTTOMRIGHT", -6, 0)
	end
	layout(false, 1, 0)

	-- Footer: the addon's memory, garbage rate and last frame, the HUD
	-- switch, the export reminder, reload.
	local footer = CreateFrame("Frame", nil, win)
	footer:SetPoint("BOTTOMLEFT", 14, 10)
	footer:SetPoint("BOTTOMRIGHT", -14, 10)
	footer:SetHeight(28)
	local footRule = K.hairline(footer, C.line)
	footRule:SetPoint("TOPLEFT", 0, 4)
	footRule:SetPoint("TOPRIGHT", 0, 4)
	local footText = K.text(footer, "numeric", 11, C.muted)
	footText:SetPoint("LEFT")
	local reload = K.button(footer, "Reload UI", function()
		ReloadUI()
	end)
	reload:SetPoint("RIGHT")
	local hudSwitch = K.toggle(footer, "HUD", function()
		if not console.hud then
			console.buildHud()
		end
		console.toggleHud()
		console.refresh(true)
	end)
	hudSwitch:SetPoint("RIGHT", reload, "LEFT", -8, 0)
	local reminder = K.text(footer, "label", 11, C.accent)
	reminder:SetPoint("RIGHT", hudSwitch, "LEFT", -12, 0)

	-- Resize grip.
	local grip = CreateFrame("Button", nil, win)
	grip:SetSize(14, 14)
	grip:SetPoint("BOTTOMRIGHT", -2, 2)
	local gripMark = K.fill(grip, C.lineStrong, "OVERLAY")
	gripMark:SetSize(6, 6)
	gripMark:SetPoint("CENTER")
	grip:SetScript("OnMouseDown", function()
		win:StartSizing("BOTTOMRIGHT")
	end)
	grip:SetScript("OnMouseUp", function()
		win:StopMovingOrSizing()
		console.saveState()
		console.refresh(true)
	end)

	-- Context menu (right-click on a value).
	local menu = CreateFrame("Frame", nil, win)
	menu:SetFrameStrata("FULLSCREEN_DIALOG")
	menu:SetSize(200, 10)
	menu:Hide()
	local menuBg = K.fill(menu, C.surface)
	menuBg:SetAllPoints()
	K.border(menu, C.line)
	local menuItems = {}
	local function openMenu(items)
		for _, b in ipairs(menuItems) do
			b:Hide()
		end
		local y = -4
		for i, item in ipairs(items) do
			local b = menuItems[i]
			if not b then
				b = CreateFrame("Button", nil, menu)
				b:SetSize(198, 24)
				local hover = K.fill(b, C.raised)
				hover:SetAllPoints()
				hover:Hide()
				b.fs = K.text(b, "label", 12, C.ink)
				b.fs:SetPoint("LEFT", 12, 0)
				b:SetScript("OnEnter", function()
					hover:Show()
				end)
				b:SetScript("OnLeave", function()
					hover:Hide()
				end)
				menuItems[i] = b
			end
			b.fs:SetText(item.label)
			b:SetScript("OnClick", function()
				menu:Hide()
				item.run()
			end)
			b:ClearAllPoints()
			b:SetPoint("TOPLEFT", 1, y)
			b:Show()
			y = y - 24
		end
		menu:SetHeight(-y + 4)
		local cx, cy = GetCursorPosition()
		local scale = plain(UIParent:GetEffectiveScale(), 1)
		menu:ClearAllPoints()
		menu:SetPoint("TOPLEFT", UIParent, "BOTTOMLEFT", cx / scale, cy / scale)
		menu:Show()
	end
	U.openMenu = openMenu

	-- A box to copy a value from (the client has no clipboard API).
	local copyBox = K.field(win, 420, "", nil)
	copyBox:SetFrameStrata("FULLSCREEN_DIALOG")
	copyBox:SetPoint("BOTTOM", footer, "TOP", 0, 8)
	copyBox:Hide()
	copyBox:SetScript("OnEditFocusLost", function(self)
		self:Hide()
	end)
	local function copy(value)
		copyBox:SetText(value)
		copyBox:Show()
		copyBox:SetFocus()
		copyBox:HighlightText()
	end
	U.copy = copy

	-- Rows: a pool of frames with cells at column offsets.
	local rows = {}
	local function row(i)
		local r = rows[i]
		if not r then
			r = CreateFrame("Button", nil, list)
			r:RegisterForClicks("LeftButtonUp", "RightButtonUp")
			r.hover = K.fill(r, C.raised)
			r.hover:SetAllPoints()
			r.hover:Hide()
			r.rule = K.hairline(r, C.line)
			r.rule:SetPoint("BOTTOMLEFT")
			r.rule:SetPoint("BOTTOMRIGHT")
			r.cells, r.subs, r.notes, r.bars = {}, {}, {}, {}
			r.badge = CreateFrame("Frame", nil, r)
			r.badge:SetSize(40, 15)
			r.badge.bg = K.fill(r.badge, C.warning)
			r.badge.bg:SetAllPoints()
			r.badge.fs = K.text(r.badge, "numeric", 10, C.onAccent)
			r.badge.fs:SetPoint("CENTER")
			r:SetScript("OnEnter", function(self)
				self.hover:Show()
			end)
			r:SetScript("OnLeave", function(self)
				self.hover:Hide()
			end)
			r:SetScript("OnClick", function(self, mouse)
				if mouse == "RightButton" and self.onRightClick then
					self.onRightClick()
				elseif mouse ~= "RightButton" and self.onClick then
					self.onClick()
				end
			end)
			rows[i] = r
		end
		return r
	end
	local function cell(r, j)
		local fs = r.cells[j]
		if not fs then
			fs = K.text(r, "numeric", 12, C.ink)
			r.cells[j] = fs
		end
		return fs
	end
	local function sub(r, j)
		local fs = r.subs[j]
		if not fs then
			fs = K.text(r, "body", 11, C.muted)
			r.subs[j] = fs
		end
		return fs
	end
	local function note(r, j)
		local fs = r.notes[j]
		if not fs then
			fs = K.text(r, "body", 11, C.muted)
			fs:SetWordWrap(true)
			fs:SetJustifyV("TOP")
			r.notes[j] = fs
		end
		return fs
	end
	-- A row's trend: up to 15 bars in a cell.
	local function bars(r, values, x0, w, h)
		local most = 0
		for _, v in ipairs(values or {}) do
			most = math.max(most, v)
		end
		for i = 1, 15 do
			local b = r.bars[i]
			if not b then
				b = K.fill(r, C.accentSoft, "ARTWORK")
				r.bars[i] = b
			end
			local v = values and values[i]
			if v and most > 0 then
				local bh = math.max(1, v / most * (h - 10))
				b:ClearAllPoints()
				b:SetPoint("BOTTOMLEFT", r, "BOTTOMLEFT", x0 + (i - 1) * (w / 15), 5)
				b:SetSize(math.max(w / 15 - 1, 1), bh)
				b:Show()
			else
				b:Hide()
			end
		end
	end
	local width = 880 - 40
	-- columns: { title, width (0 = rest), align, sort key }; data rows:
	-- { cells = {}, subs = {}, colors = {}, fonts = {}, badge, indent,
	--   tall, notes = {}, trend = {} (in the column `trendColumn`),
	--   onClick, onRightClick }.
	local function render(columns, data, trendColumn)
		width = plain(scroll:GetWidth(), 880 - 40)
		list:SetWidth(width)
		local fixed = 0
		for _, c in ipairs(columns) do
			do local __wl_t5 = c.width if __wl_t5 == nil then __wl_t5 = 0 end fixed = fixed + (__wl_t5) + 12 end
		end
		local y = 0
		local function colWidth(j)
			local w = columns[j].width
			return (w and w > 0) and w or math.max(width - fixed - 16, 60)
		end
		local function x0(j)
			local xpos = 8
			for k = 1, j - 1 do
				xpos = xpos + colWidth(k) + 12
			end
			return xpos
		end
		-- Header row.
		local hr = row(1)
		hr:ClearAllPoints()
		hr:SetPoint("TOPLEFT", 0, 0)
		hr:SetSize(width, 24)
		hr.onClick, hr.onRightClick = nil, nil
		hr.badge:Hide()
		hr.rule:SetColorTexture(unpack(C.lineStrong))
		bars(hr, nil, 0, 0, 0)
		for j, c in ipairs(columns) do
			local fs = cell(hr, j)
			K.font(fs, "label", 11)
			local active = c.sort and c.sort == S.sort
			fs:SetTextColor(unpack(active and C.accent or C.muted))
			fs:SetText(c.title)
			fs:ClearAllPoints()
			fs:SetPoint("LEFT", hr, "LEFT", x0(j), 0)
			fs:SetWidth(colWidth(j))
			fs:SetJustifyH(c.align or "LEFT")
			fs:Show()
		end
		for j = #columns + 1, #hr.cells do
			hr.cells[j]:Hide()
		end
		for _, fs in ipairs(hr.subs) do
			fs:Hide()
		end
		for _, fs in ipairs(hr.notes) do
			fs:Hide()
		end
		hr.onClick = function()
			-- Sorting: the column under the cursor.
			local cx = GetCursorPosition() / plain(UIParent:GetEffectiveScale(), 1) - plain(hr:GetLeft(), 0)
			for j, c in ipairs(columns) do
				if c.sort and cx >= x0(j) and cx <= x0(j) + colWidth(j) then
					S.sort = c.sort
					console.refresh(true)
				end
			end
		end
		hr:Show()
		y = 24
		for i, d in ipairs(data) do
			local r = row(i + 1)
			local notes = d.notes or {}
			local h = (d.tall and 34 or 24) + #notes * 28
			r:ClearAllPoints()
			r:SetPoint("TOPLEFT", 0, -y)
			r:SetSize(width, h)
			r.rule:SetColorTexture(unpack(C.line))
			r.onClick, r.onRightClick = d.onClick, d.onRightClick
			for j = 1, #columns do
				local fs = cell(r, j)
				do local __wl_t6 = d.fonts if __wl_t6 ~= nil then __wl_t6 = __wl_t6[j] end local __wl_t7 = __wl_t6 if __wl_t7 == nil then __wl_t7 = (j == 1 and "label" or "numeric") end K.font(fs, __wl_t7, 12) end
				do local __wl_t8 = d.colors if __wl_t8 ~= nil then __wl_t8 = __wl_t8[j] end local __wl_t9 = __wl_t8 if __wl_t9 == nil then __wl_t9 = C.ink end fs:SetTextColor(unpack(__wl_t9)) end
				fs:SetText(d.cells[j] or "")
				fs:ClearAllPoints()
				local indent = (j == 1 and d.indent) or 0
				fs:SetPoint("TOPLEFT", r, "TOPLEFT", x0(j) + indent, d.tall and -4 or -5)
				fs:SetWidth(math.max(colWidth(j) - indent, 10))
				fs:SetJustifyH(columns[j].align or "LEFT")
				fs:SetShown(j ~= trendColumn)
				local s = sub(r, j)
				if d.subs and d.subs[j] then
					s:SetText(d.subs[j])
					s:ClearAllPoints()
					s:SetPoint("TOPLEFT", fs, "BOTTOMLEFT", 0, -2)
					s:SetWidth(math.max(colWidth(j) - indent, 10))
					s:Show()
				else
					s:Hide()
				end
			end
			for j = #columns + 1, #r.cells do
				r.cells[j]:Hide()
				if r.subs[j] then
					r.subs[j]:Hide()
				end
			end
			for j, text in ipairs(notes) do
				local fs = note(r, j)
				fs:SetText(text)
				do local __wl_t10 = d.noteColors if __wl_t10 ~= nil then __wl_t10 = __wl_t10[j] end local __wl_t11 = __wl_t10 if __wl_t11 == nil then __wl_t11 = C.muted end fs:SetTextColor(unpack(__wl_t11)) end
				fs:ClearAllPoints()
				do local __wl_t12 = d.indent if __wl_t12 == nil then __wl_t12 = 0 end fs:SetPoint("TOPLEFT", r, "TOPLEFT", 8 + (__wl_t12), -(d.tall and 34 or 24) - (j - 1) * 28) end
				do local __wl_t13 = d.indent if __wl_t13 == nil then __wl_t13 = 0 end fs:SetSize(width - 24 - (__wl_t13), 26) end
				fs:Show()
			end
			for j = #notes + 1, #r.notes do
				r.notes[j]:Hide()
			end
			if trendColumn then
				bars(r, d.trend, x0(trendColumn), colWidth(trendColumn), h)
			else
				bars(r, nil, 0, 0, 0)
			end
			if d.badge then
				r.badge.fs:SetText(d.badge.text)
				r.badge.bg:SetColorTexture(unpack(d.badge.color or C.warning))
				r.badge:SetWidth(plain(r.badge.fs:GetStringWidth(), 30) + 10)
				r.badge:ClearAllPoints()
				r.badge:SetPoint("TOPRIGHT", r, "TOPRIGHT", -8, -5)
				r.badge:Show()
			else
				r.badge:Hide()
			end
			r:Show()
			y = y + h
		end
		for i = #data + 2, #rows do
			rows[i]:Hide()
		end
		list:SetHeight(math.max(y, 1))
		layoutThumb()
	end
	U.render = render

	-- Toolbar widgets per tab (created once, shown when their tab is open).
	local tools = {}
	local function tool(name, make)
		if not tools[name] then
			tools[name] = make()
		end
		return tools[name]
	end
	local function showTools(names)
		for name, w in pairs(tools) do
			local shown = false
			for _, n in ipairs(names) do
				shown = shown or n == name
			end
			w:SetShown(shown)
		end
	end
	U.tool, U.showTools, U.toolbar, U.toolbar2 = tool, showTools, toolbar, toolbar2

	-- The canvas: pooled textures, labels and click areas, cleared per draw.
	local paint = {tex = {}, fs = {}, nt = 0, nf = 0}
	local function clearCanvas()
		for i = 1, paint.nt do
			paint.tex[i]:Hide()
		end
		for i = 1, paint.nf do
			paint.fs[i]:Hide()
		end
		paint.nt, paint.nf = 0, 0
	end
	local function rect(xpos, ypos, w, h, color)
		paint.nt = paint.nt + 1
		local t = paint.tex[paint.nt]
		if not t then
			t = K.fill(canvas, color, "ARTWORK")
			paint.tex[paint.nt] = t
		end
		t:SetColorTexture(unpack(color))
		t:ClearAllPoints()
		t:SetPoint("BOTTOMLEFT", canvas, "BOTTOMLEFT", xpos, ypos)
		t:SetSize(math.max(w, 1), math.max(h, 1))
		t:Show()
		return t
	end
	local function label(xpos, ypos, s, color, w)
		paint.nf = paint.nf + 1
		local fs = paint.fs[paint.nf]
		if not fs then
			fs = K.text(canvas, "body", 10, C.muted)
			paint.fs[paint.nf] = fs
		end
		fs:SetTextColor(unpack(color or C.muted))
		fs:SetText(s)
		fs:ClearAllPoints()
		fs:SetPoint("BOTTOMLEFT", canvas, "BOTTOMLEFT", xpos, ypos)
		fs:SetWidth(w or 0)
		fs:Show()
		return fs
	end
	U.clearCanvas, U.rect, U.label, U.canvas = clearCanvas, rect, label, canvas
	-- Where the cursor is on the canvas (x from the left, y from the bottom).
	function U.cursor()
		local cx, cy = GetCursorPosition()
		local scale = plain(canvas:GetEffectiveScale(), 1)
		return cx / scale - plain(canvas:GetLeft(), 0), cy / scale - plain(canvas:GetBottom(), 0)
	end
	canvas:SetScript("OnClick", function(_, mouse)
		if U.canvasClick then
			U.canvasClick(mouse)
		end
	end)
	canvas:SetScript("OnMouseWheel", function(_, delta)
		if U.canvasWheel then
			U.canvasWheel(delta)
		end
	end)
	recPanel:SetScript("OnMouseWheel", function(_, delta)
		S.recOffset = math.max(0, S.recOffset - delta)
		console.refresh(true)
	end)

	-- Inspector: the value shown and how to reach it again.
	local inspected = {label = "ns", get = function()
		local d = current()
		do local __wl_t14 = d if __wl_t14 ~= nil then __wl_t14 = __wl_t14.runtime.ns end return __wl_t14 end
	end}
	local function inspect(name, get)
		inspected = {label = name, get = get}
		S.expanded = {[""] = true}
		S.tab = 7
		console.refresh(true)
	end
	local function addWatch(name, fn, source)
		local d = current()
		if d then
			d.watches[#d.watches + 1] = {label = name, fn = fn, source = source}
		end
		S.tab = 6
		console.refresh(true)
	end

	local views = {}
	U.views = views
	function views.overview(d)
		layout(false, 1, 90)
		showTools({})
		local data = {}
		for _, k in ipairs({"version", "profile", "build", "flavors", "compiler"}) do
			data[#data + 1] = {cells = {k, tostring(d.info[k] or "?")}, colors = {C.muted, C.ink}}
		end
		local rt = d.runtime
		local toolRows = {
		{"Profiler", rt.prof and "on" or "off"},
		{"Monitor", rt.mon and "events, scripts, timers, memory" or "off"},
		{"Recordings", rt.perf and "frame time, slow calls, garbage" or "off"},
		{"Error capture", rt.errors and "on" or "off"},
		{"Log", rt.logs and "wowlua:log" or "not used"}
		}
		for _, t in ipairs(toolRows) do
			data[#data + 1] = {cells = {t[1], t[2]}, colors = {C.ink, C.muted}, fonts = {"label", "body"}}
		end
		-- wowlua:meta's values.
		local meta = rt.meta
		local keys = {}
		for kind, t in pairs(meta or {}) do
			if type(t) == "table" then
				for k, v in pairs(t) do
					if type(v) ~= "table" then
						keys[#keys + 1] = {tostring(kind) .. "." .. tostring(k), tostring(v)}
					end
				end
			end
		end
		table.sort(keys, function(a, b)
			return a[1] < b[1]
		end)
		for _, kv in ipairs(keys) do
			data[#data + 1] = {cells = kv, colors = {C.muted, C.ink}, fonts = {"body", "body"}}
		end
		-- Memory over time.
		local __wl_t15 = rt.mon if __wl_t15 ~= nil then __wl_t15 = __wl_t15.memory end local __wl_t16 = __wl_t15 if __wl_t16 == nil then __wl_t16 = {} end local mem = __wl_t16
		local last = mem[#mem]
		data[#data + 1] = {
		cells = {"Memory", last and string.format("%.2f MB · %d samples, every 5 s", last / 1024, #mem) or "no samples yet"},
		colors = {C.ink, C.ink}
		}
		clearCanvas()
		local cw = plain(canvas:GetWidth(), 600)
		local most = 0
		for _, v in ipairs(mem) do
			most = math.max(most, v)
		end
		local bw = cw / 120
		for i, v in ipairs(mem) do
			rect(4 + (i - 1) * bw, 4, math.max(bw - 1, 1), most > 0 and v / most * 70 or 1, C.noteSoft)
		end
		label(8, 74, last and ("memory, last 10 minutes · " .. K.kb(most)) or "memory, last 10 minutes: sampled every 5 seconds outside combat", C.muted, cw - 16)
		U.canvasClick, U.canvasWheel = nil, nil
		render({{title = "Build", width = 200}, {title = "", width = 0}}, data)
	end
	function views.monitor(d)
		layout(false, 1, 0)
		local seg = tool("monSeg", function()
			local s = K.segmented(toolbar, {
			{value = "events", label = "Events"},
			{value = "scripts", label = "Scripts"},
			{value = "timers", label = "Timers"},
			{value = "commands", label = "Commands"}
			}, function(v)
				S.monitor = v
				console.refresh(true)
			end)
			s:SetPoint("LEFT")
			return s
		end)
		seg:Select(S.monitor)
		tool("monReset", function()
			local b = K.button(toolbar, "Reset", function()
				local cur = current()
				if cur and cur.runtime.mon then
					cur.runtime.mon.reset()
				end
				console.refresh(true)
			end)
			b:SetPoint("RIGHT")
			return b
		end)
		showTools({"monSeg", "monReset"})
		local mon = d.runtime.mon
		local data = {}
		if mon then
			for _, r in ipairs(console.sorted(mon[S.monitor] or {}, "total")) do
				local s = r.s
				local __wl_t17 = s.max if __wl_t17 == nil then __wl_t17 = 0 end local slow = (__wl_t17) > 16.7
				data[#data + 1] = {
				cells = {r.name, K.count(s.count), K.ms(s.total), K.ms(s.max)},
				subs = {s.detail},
				colors = {C.ink, C.ink, C.ink, slow and C.warning or C.ink},
				tall = s.detail ~= nil
				}
			end
		else
			data[1] = {cells = {"The monitor is off in this build"}, colors = {C.muted}}
		end
		local head = ({events = "Event", scripts = "Frame · script", timers = "Timer", commands = "Slash command"})[S.monitor]
		render({
		{title = head, width = 0},
		{title = S.monitor == "events" and "Count" or "Runs", width = 72, align = "RIGHT"},
		{title = "Total ms", width = 84, align = "RIGHT"},
		{title = "Max ms", width = 84, align = "RIGHT"}
		}, data)
	end
	local LEVEL_RANK = {trace = 0, debug = 1, info = 2, warn = 3, error = 4}
	function views.log(d)
		layout(false, 1, 0)
		local seg = tool("logSeg", function()
			local s = K.segmented(toolbar, {
			{value = "all", label = "All"},
			{value = "info", label = "Info"},
			{value = "warn", label = "Warn"},
			{value = "error", label = "Error"}
			}, function(v)
				S.level = v
				console.refresh(true)
			end)
			s:SetPoint("LEFT")
			return s
		end)
		seg:Select(S.level)
		showTools({"logSeg"})
		local logs = d.runtime.logs
		local data = {}
		if logs then
			local min = S.level == "all" and -1 or LEVEL_RANK[S.level]
			for _, e in ipairs(logs.history()) do repeat
					do local __wl_t18 = LEVEL_RANK[e.level] if __wl_t18 == nil then __wl_t18 = 0 end if (__wl_t18) < min then
							break
						end end
					local tone = e.level == "error" and C.error or (e.level == "warn" and C.warning or C.muted)
					do
						local __wl_t19 = e.time if __wl_t19 == nil then __wl_t19 = 0 end local __wl_t21 = string.format("%.1f", __wl_t19) local __wl_t20 = e.location if __wl_t20 == nil then __wl_t20 = "" end data[#data + 1] = {cells = {__wl_t21, e.level, e.message, __wl_t20},
						colors = {C.muted, tone, C.ink, C.muted},
						fonts = {"numeric", "label", "body", "body"}
						} end
				until true end
		else
			data[1] = {cells = {"", "", "This addon does not use wowlua:log"}, colors = {C.ink, C.ink, C.muted}}
		end
		render({
		{title = "Time", width = 64},
		{title = "Level", width = 56},
		{title = "Message", width = 0},
		{title = "Location", width = 180}
		}, data)
	end
	function views.errors(d)
		layout(false, 1, 0)
		tool("errClear", function()
			local b = K.button(toolbar, "Clear", function()
				local cur = current()
				if cur and cur.runtime.errors then
					cur.runtime.errors.clear()
				end
				console.refresh(true)
			end)
			b:SetPoint("RIGHT")
			return b
		end)
		showTools({"errClear"})
		local errors = d.runtime.errors
		local data = {}
		if errors then
			for _, e in ipairs(errors.list) do
				local open = S.openError == e
				data[#data + 1] = {
				cells = {(open and "- " or "+ ") .. e.message, "×" .. e.count, e.time},
				colors = {C.error, C.ink, C.muted},
				onClick = function()
					S.openError = (not open) and e or nil
					console.refresh(true)
				end,
				onRightClick = function()
					openMenu({{label = "Copy", run = function()
						copy(e.message .. "\n" .. (e.stack or ""))
					end}})
				end
				}
				if open then
					for line in tostring(e.stack or ""):gmatch("[^\n]+") do
						data[#data + 1] = {cells = {line}, colors = {C.ink}, fonts = {"body"}, indent = 20}
					end
				end
			end
			if #errors.list == 0 then
				data[1] = {cells = {"No errors"}, colors = {C.muted}}
			end
		else
			data[1] = {cells = {"Error capture is off in this build"}, colors = {C.muted}}
		end
		render({{title = "Error", width = 0}, {title = "", width = 48, align = "RIGHT"}, {title = "", width = 70, align = "RIGHT"}}, data)
	end
	local function valueText(ok, v)
		if not ok then
			return tostring(v), C.error
		end
		if type(v) == "table" and not (issecretvalue and issecretvalue(v)) then
			local n = 0
			for _ in pairs(v) do
				n = n + 1
			end
			return "table: " .. n .. " fields", C.note
		end
		local shown = console.show(v)
		return shown, shown == "<secret>" and C.muted or C.ink
	end
	function views.watch(d)
		layout(false, 1, 0)
		local input = tool("watchInput", function()
			local f = K.field(toolbar, 520, "Lua expression: ns.db.profile.scale, UnitHealth(\"player\")", function(expression)
				local cur = current()
				if not cur or expression == "" then
					return
				end
				local fn, problem = console.compile(cur, expression)
				if fn then
					addWatch(expression, fn, "typed")
				else
					console.say(tostring(problem))
				end
			end)
			f:SetPoint("LEFT")
			return f
		end)
		tool("watchAdd", function()
			local b = K.button(toolbar, "Watch", function()
				input:GetScript("OnEnterPressed")(input)
			end, true)
			b:SetPoint("LEFT", input, "RIGHT", 8, 0)
			return b
		end)
		showTools({"watchInput", "watchAdd"})
		local data = {}
		for i, w in ipairs(d.watches) do
			local ok, v = pcall(w.fn)
			local shown, color = valueText(ok, v)
			local function remove()
				table.remove(d.watches, i)
				console.refresh(true)
			end
			data[#data + 1] = {
			cells = {w.label, shown, w.source},
			colors = {C.ink, color, C.muted},
			fonts = {"label", "numeric", "body"},
			onClick = function()
				if ok and type(v) == "table" then
					inspect(w.label, w.fn)
				end
			end,
			onRightClick = function()
				openMenu({
				{label = "Inspect", run = function()
					inspect(w.label, w.fn)
				end},
				{label = "Copy value", run = function()
					copy(shown)
				end},
				{label = "Remove", run = remove}
				})
			end
			}
		end
		if #data == 0 then
			data[1] = {cells = {"No watches. Add one above, with debug.watch, or /wl watch."}, colors = {C.muted}, fonts = {"body"}}
		end
		render({{title = "Watch", width = 0}, {title = "Value", width = 280}, {title = "From", width = 120}}, data)
	end
	function views.inspector(_)
		layout(false, 1, 0)
		showTools({})
		local data = {}
		local ok, root = pcall(inspected.get)
		local function walk(value, path, name, depth, seen, getter)
			local open = S.expanded[path]
			local isTable = type(value) == "table" and not (issecretvalue and issecretvalue(value))
			local shown, color = valueText(true, value)
			if isTable and seen[value] then
				shown, color = "(cycle)", C.muted
				isTable = false
			end
			data[#data + 1] = {
			cells = {(isTable and (open and "- " or "+ ") or "   ") .. name, shown, type(value)},
			colors = {name:sub(1, 1) == "<" and C.muted or C.ink, color, C.muted},
			fonts = {"label", "numeric", "body"},
			indent = depth * 18,
			onClick = function()
				if isTable then
					S.expanded[path] = not open
					console.refresh(true)
				end
			end,
			onRightClick = function()
				openMenu({
				{label = "Watch", run = function()
					addWatch(inspected.label .. path, getter, "from inspector")
				end},
				{label = "Copy value", run = function()
					copy(shown)
				end}
				})
			end
			}
			if isTable and open and depth < 8 then
				seen[value] = true
				local keys = {}
				for k in pairs(value) do
					keys[#keys + 1] = k
				end
				table.sort(keys, function(a, b)
					return tostring(a) < tostring(b)
				end)
				for _, k in ipairs(keys) do
					local key = k
					local suffix = type(k) == "string" and k:match("^[%a_][%w_]*$") and ("." .. k) or ("[" .. tostring(k) .. "]")
					walk(value[k], path .. suffix, tostring(k), depth + 1, seen, function()
						return getter()[key]
					end)
				end
				local mt = getmetatable(value)
				if type(mt) == "table" then
					walk(mt, path .. "<metatable>", "<metatable>", depth + 1, seen, function()
						return getmetatable(getter())
					end)
				end
				seen[value] = nil
			end
		end
		if ok then
			if S.expanded[""] == nil then
				S.expanded[""] = true
			end
			walk(root, "", inspected.label, 0, {}, inspected.get)
		else
			data[1] = {cells = {inspected.label, tostring(root)}, colors = {C.ink, C.error}}
		end
		render({{title = "Key", width = 0}, {title = "Value", width = 300}, {title = "Type", width = 80}}, data)
	end
	function views.hints(d)
		layout(false, 1, 0)
		tool("hintsRestore", function()
			local b = K.button(toolbar, "Show dismissed", function()
				local cur = current()
				if cur and cur.hintState then
					cur.hintState.dismissed = {}
					runHints(cur)
				end
				console.refresh(true)
			end)
			b:SetPoint("RIGHT")
			return b
		end)
		tool("hintsInfo", function()
			local fs = K.text(toolbar, "body", 11, C.muted)
			fs:SetPoint("LEFT")
			fs:SetText("Checked every 5 seconds against the live recording while the window is open")
			return fs
		end)
		showTools({"hintsRestore", "hintsInfo"})
		local h = d.hintState
		local data = {}
		if not d.runtime.perf then
			data[1] = {cells = {"This build records no performance data (profile: instrument or monitor)"}, colors = {C.muted}, fonts = {"body"}}
		end
		for _, x in ipairs(h and h.list or {}) do
			local fresh = not h.seen[x.id]
			data[#data + 1] = {
			cells = {"hint[" .. tostring(x.code) .. "] " .. tostring(x.subject), x.recording},
			colors = {C.warning, C.muted},
			fonts = {"label", "body"},
			notes = {x.evidence, "Fix: " .. x.fix},
			noteColors = {C.ink, C.muted},
			badge = fresh and {text = "new", color = C.accent} or nil,
			onClick = function()
				openMenu({
				{label = "Dismiss", run = function()
					console.dismissHint(d, x)
				end},
				{label = "Copy", run = function()
					copy("hint[" .. tostring(x.code) .. "] " .. tostring(x.subject) .. ": " .. tostring(x.evidence) .. ". " .. tostring(x.fix) .. ". (wowlua explain " .. tostring(x.code) .. ")")
				end}
				})
			end
			}
		end
		if #data == 0 then
			data[1] = {cells = {"No hints for the live recording"}, colors = {C.muted}, fonts = {"body"}}
		end
		render({{title = "Hint", width = 0}, {title = "Recording", width = 120}}, data)
		-- Seen once shown.
		for _, x in ipairs(h and h.list or {}) do
			h.seen[x.id] = true
		end
	end

	function console.refresh(_)
		local d = current()
		if not d then
			return
		end
		selected = d.name
		addonLabel:SetText(d.name)
		local fresh = newHints(d)
		for i, b in ipairs(tabButtons) do
			local active = i == S.tab
			b.fs:SetTextColor(unpack(active and C.ink or C.muted))
			b.line:SetShown(active)
			if i == 8 then
				b.badge:SetShown(fresh > 0 and not active)
				b.badge.fs:SetText(tostring(fresh))
				b.badge:SetWidth(math.max(16, plain(b.badge.fs:GetStringWidth(), 8) + 8))
			end
		end
		layoutTabs()
		-- Footer: memory, garbage rate, the last frame.
		local rt = d.runtime
		local parts = {}
		local __wl_t22 = rt.mon if __wl_t22 ~= nil then __wl_t22 = __wl_t22.memory end local mem = __wl_t22
		if mem and #mem > 0 then
			parts[#parts + 1] = "memory " .. K.kb(mem[#mem])
		end
		local perf = rt.perf
		if perf then
			local sec = perf.seconds
			local gc = sec.garbage[sec.last]
			do local __wl_t23 = gc if __wl_t23 == nil then __wl_t23 = 0 end parts[#parts + 1] = "garbage " .. K.kb(__wl_t23) .. "/s" end
			parts[#parts + 1] = "frame " .. K.ms(perf.lastFrame) .. " ms"
		end
		parts[#parts + 1] = (d.info.version or "?") .. " · " .. (d.info.profile or "?")
		footText:SetText(table.concat(parts, "  ·  "))
		-- The export reminder: marked recordings are saved on /reload.
		local changed, marked = false, 0
		for _, n in ipairs(reg.order) do
			local p = (reg.addons[n]).runtime.perf
			if p and p.exportPending then
				changed = true
				for _, rec in ipairs(p.recordings) do
					if rec.export then
						marked = marked + 1
					end
				end
			end
		end
		reminder:SetText(changed and marked .. " marked for export · /reload saves them" or "")
		hudSwitch:SetOn(console.hudShown and console.hudShown() or false)
		views[VIEW_KEYS[S.tab] or "overview"](d)
	end

	-- A dismissed hint comes back when its number grows by half.
	function console.dismissHint(d, x)
		d.hintState.dismissed[x.id] = x.metric
		runHints(d)
		console.refresh(true)
	end
	function console.setTab(i)
		S.tab = i
		console.refresh()
	end
	-- Opens the window on `name`'s tab `tab`, with `e` expanded (errors).
	function console.openOn(name, tab, e)
		if name and reg.addons[name] then
			selected = name
		end
		if tab then
			S.tab = tab
		end
		if e then
			S.openError = e
		end
		if not K.isShown(win) then
			win:Show()
		end
		console.refresh(true)
	end
	U.S, U.tabButtons = S, tabButtons

	function console.saveState()
		local point, _, _, px, py = win:GetPoint(1)
		local saved = {
		point = K.plainText(point), x = plain(px, 0), y = plain(py, 0),
		width = plain(win:GetWidth(), 880), height = plain(win:GetHeight(), 640),
		tab = S.tab, addon = selected, view = S.view
		}
		for _, n in ipairs(reg.order) do
			devDB(n).window = saved
		end
	end
	for _, n in ipairs(reg.order) do
		local saved = devDB(n).window
		if type(saved) == "table" then
			S.tab = saved.tab or S.tab
			S.view = saved.view or S.view
			selected = (saved.addon and reg.addons[saved.addon]) and saved.addon or selected
			if saved.point and saved.point ~= "" then
				win:ClearAllPoints()
				do local __wl_t24 = saved.x if __wl_t24 == nil then __wl_t24 = 0 end local __wl_t25 = saved.y if __wl_t25 == nil then __wl_t25 = 0 end win:SetPoint(saved.point, UIParent, saved.point, __wl_t24, __wl_t25) end
			end
			if saved.width and saved.height then
				win:SetSize(saved.width, saved.height)
			end
			break
		end
	end

	-- Refreshed only while visible: watches and the inspector often; the
	-- hints every 5 seconds.
	local elapsed, sinceHints = 0, 5
	win:SetScript("OnUpdate", function(_, dt)
		elapsed = elapsed + (dt or 0)
		sinceHints = sinceHints + (dt or 0)
		if sinceHints >= 5 then
			sinceHints = 0
			local d = current()
			if d then
				runHints(d)
			end
		end
		local every = (S.tab == 6 or S.tab == 7) and 0.25 or 1
		if elapsed >= every and not K.isShown(menu) then
			elapsed = 0
			console.refresh()
		end
	end)
	win:SetScript("OnShow", function()
		sinceHints = 5
		if console.updateHud then
			console.updateHud()
		end
		if console.hideToast then
			console.hideToast()
		end
	end)
	win:SetScript("OnHide", function()
		menu:Hide()
		addonList:Hide()
		console.saveState()
		if console.updateHud then
			console.updateHud()
		end
	end)
	console.U = U
	console.buildPerformance(U, PERF_VIEWS, layout)
	return win
end

-- The performance tab (§7.7.4): recordings on the left, eight views on the
-- right, comparing, switches and thresholds, export.
local function buildPerformance(console, U, PERF_VIEWS, layout)
	local C, plain, S = K.C, K.plain, U.S
	local tool, showTools, render = U.tool, U.showTools, U.render
	local toolbar, toolbar2 = U.toolbar, U.toolbar2
	local views = U.views
	local sub = {}

	-- The recordings list.
	local recRows = {}
	local function recRow(i)
		local b = recRows[i]
		if not b then
			b = CreateFrame("Button", nil, U.recPanel)
			b:RegisterForClicks("LeftButtonUp", "RightButtonUp")
			b:SetHeight(36)
			b.on = K.fill(b, C.selected)
			b.on:SetAllPoints()
			b.bar = K.fill(b, C.accent, "ARTWORK")
			b.bar:SetWidth(2)
			b.bar:SetPoint("TOPLEFT")
			b.bar:SetPoint("BOTTOMLEFT")
			b.fs = K.text(b, "label", 12, C.ink)
			b.fs:SetPoint("TOPLEFT", 10, -4)
			b.fs:SetPoint("TOPRIGHT", -8, -4)
			b.info = K.text(b, "body", 10, C.muted)
			b.info:SetPoint("TOPLEFT", 10, -20)
			b.info:SetPoint("TOPRIGHT", -8, -20)
			b:SetScript("OnClick", function(self, mouse)
				if mouse == "RightButton" then
					self.onRightClick()
				else
					self.onClick()
				end
			end)
			recRows[i] = b
		end
		return b
	end
	local function selectedRec(perf, d)
		local rec = S.recOf[d.name]
		for _, r in ipairs(perf.recordings) do
			if r == rec then
				return r
			end
		end
		return perf.recordings[1]
	end
	local function compareRec(perf, d, rec)
		local cmp = S.compareOf[d.name]
		if cmp == rec then
			return nil
		end
		for _, r in ipairs(perf.recordings) do
			if r == cmp then
				return r
			end
		end
		return nil
	end
	local function drawRecordings(d, perf, rec, cmp)
		local y = -4
		local shown = 0
		local fits = math.max(1, math.floor((plain(U.recPanel:GetHeight(), 500) - 44) / 38))
		S.recOffset = math.min(S.recOffset, math.max(0, #perf.recordings - fits))
		for i = 1, #recRows do
			recRows[i]:Hide()
		end
		for i, r in ipairs(perf.recordings) do repeat
				if i <= S.recOffset or shown >= fits then
					break
				end
				shown = shown + 1
				local b = recRow(shown)
				local data = r.data
				local length = data and data.duration or ((GetTime() - r.at) + 0)
				local what = r.kind == "pull" and (r.detail or "boss pull") or r.kind
				local marks = {}
				if r.running and r.kind ~= "live" then
					marks[#marks + 1] = "recording"
				end
				if r.export then
					marks[#marks + 1] = "↑ export"
				end
				if r == cmp then
					marks[#marks + 1] = "compared"
				end
				b.fs:SetText(r.name)
				b.fs:SetTextColor(unpack(r == rec and C.ink or C.muted))
				b.info:SetText(what .. " · " .. K.duration(length) .. (#marks > 0 and (" · " .. table.concat(marks, " · ")) or ""))
				b.info:SetTextColor(unpack(r.running and r.kind ~= "live" and C.error or C.faint))
				b.on:SetShown(r == rec)
				b.bar:SetShown(r == rec or r == cmp)
				b.bar:SetColorTexture(unpack(r == cmp and C.note or C.accent))
				b.onClick = function()
					S.recOf[d.name] = r
					console.refresh(true)
				end
				b.onRightClick = function()
					local items = {}
					if r ~= rec then
						items[#items + 1] = {label = "Compare with this", run = function()
							S.compareOf[d.name] = r
							console.refresh(true)
						end}
					end
					if cmp then
						items[#items + 1] = {label = "Stop comparing", run = function()
							S.compareOf[d.name] = nil
							console.refresh(true)
						end}
					end
					items[#items + 1] = {label = r.export and "Unmark for export" or "Mark for export", run = function()
						perf.setExport(r, not r.export)
						console.refresh(true)
					end}
					if r.running and r.kind ~= "live" then
						items[#items + 1] = {label = "Stop", run = function()
							perf.stop(r)
							console.refresh(true)
						end}
					end
					if r.kind ~= "live" and not r.running then
						items[#items + 1] = {label = "Delete", run = function()
							for j, x in ipairs(perf.recordings) do
								if x == r then
									table.remove(perf.recordings, j)
									break
								end
							end
							if r.export then
								perf.setExport(r, false)
							end
							console.refresh(true)
						end}
					end
					U.openMenu(items)
				end
				b:ClearAllPoints()
				b:SetPoint("TOPLEFT", 1, y)
				b:SetPoint("TOPRIGHT", -1, y)
				b:Show()
				y = y - 38
			until true end
		-- Manual recordings: named when stopped.
		local manual = perf.running("manual")
		local nameField = tool("recName", function()
			local f = K.field(U.recPanel, 110, "name", nil)
			f:SetPoint("BOTTOMLEFT", 6, 6)
			return f
		end)
		local recButton = tool("recButton", function()
			local b = K.button(U.recPanel, "Record", function()
				local cur = U.current()
				local p = cur and cur.runtime.perf
				if not p then
					return
				end
				local m = p.running("manual")
				if m then
					p.stop(m, K.plainText(nameField:GetText()))
					nameField:SetText("")
					S.recOf[cur.name] = m
				else
					S.recOf[cur.name] = p.start("manual")
				end
				console.refresh(true)
			end, true)
			b:SetPoint("BOTTOMRIGHT", -6, 6)
			return b
		end)
		recButton:SetLabel(manual and "Stop" or "Record")
		nameField:SetShown(manual ~= nil)
		recButton:Show()
	end

	-- The switches and thresholds (§7.7.5), and comparing.
	local function settingsBar(d, perf, cmp)
		local paths = tool("swPaths", function()
			local b = K.toggle(toolbar2, "Call paths", function()
				local __wl_t26 = U.current() if __wl_t26 ~= nil then __wl_t26 = __wl_t26.runtime.perf end local p = __wl_t26
				if p then
					p.set("callPaths", not p.callPaths)
				end
				console.refresh(true)
			end)
			b:SetPoint("LEFT")
			return b
		end)
		local gc = tool("swGarbage", function()
			local b = K.toggle(toolbar2, "Garbage per function", function()
				local __wl_t27 = U.current() if __wl_t27 ~= nil then __wl_t27 = __wl_t27.runtime.perf end local p = __wl_t27
				if p then
					p.set("garbage", not p.garbage)
				end
				console.refresh(true)
			end)
			b:SetPoint("LEFT", paths, "RIGHT", 6, 0)
			return b
		end)
		local function threshold(name, key, anchor, caption)
			return tool(name, function()
				local cap = K.text(toolbar2, "body", 11, C.muted)
				cap:SetPoint("LEFT", anchor, "RIGHT", 12, 0)
				cap:SetText(caption)
				local f = K.field(toolbar2, 52, "ms", function(text)
					local __wl_t28 = U.current() if __wl_t28 ~= nil then __wl_t28 = __wl_t28.runtime.perf end local p = __wl_t28
					local v = tonumber(text)
					if p and v and v > 0 then
						p.set(key, v)
					end
					console.refresh(true)
				end)
				f:SetPoint("LEFT", cap, "RIGHT", 6, 0)
				f.cap = cap
				local base = f.SetShown
				function f:SetShown(on)
					base(self, on)
					cap:SetShown(on)
				end
				return f
			end)
		end
		local budget = threshold("swBudget", "frameBudget", gc, "budget")
		local slow = threshold("swSlow", "slowCall", budget, "slow call")
		tool("swReset", function()
			local b = K.button(toolbar2, "Profile's", function()
				local __wl_t29 = U.current() if __wl_t29 ~= nil then __wl_t29 = __wl_t29.runtime.perf end local p = __wl_t29
				if p then
					p.resetSettings()
				end
				console.refresh(true)
			end)
			b:SetPoint("LEFT", slow, "RIGHT", 6, 0)
			return b
		end)
		local vs = tool("swCompare", function()
			local b = K.button(toolbar2, "vs", function()
				local cur = U.current()
				if cur then
					S.compareOf[cur.name] = nil
				end
				console.refresh(true)
			end)
			b:SetPoint("RIGHT")
			return b
		end)
		paths:SetOn(perf.callPaths)
		gc:SetOn(perf.garbage)
		if budget:HasFocus() ~= true then
			budget:SetText(tostring(perf.frameBudget))
		end
		if slow:HasFocus() ~= true then
			slow:SetText(tostring(perf.slowCall))
		end
		vs:SetLabel(cmp and ("vs " .. cmp.name .. " ×") or "")
		vs:SetShown(cmp ~= nil)
		local _ = d
		return cmp ~= nil
	end

	function views.performance(d)
		local perf = d.runtime.perf
		if not perf then
			layout(false, 1, 0)
			showTools({})
			U.clearCanvas()
			render({{title = "Performance", width = 0}}, {
			{cells = {"This build records no performance data: it needs a profile with instrument or monitor"}, colors = {C.muted}, fonts = {"body"}}
			})
			return
		end
		local rec = selectedRec(perf, d)
		local cmp = compareRec(perf, d, rec)
		local seg = tool("perfViews", function()
			local s = K.segmented(toolbar, PERF_VIEWS, function(v)
				S.view = v
				console.saveState()
				console.refresh(true)
			end)
			s:SetPoint("LEFT")
			return s
		end)
		seg:Select(S.view)
		local data = perf.data(rec)
		local other = cmp and perf.data(cmp)
		local names = {"perfViews", "recName", "recButton"}
		if S.view ~= "bench" then
			for _, n in ipairs({"swPaths", "swGarbage", "swBudget", "swSlow", "swReset"}) do
				names[#names + 1] = n
			end
			if cmp then
				names[#names + 1] = "swCompare"
			end
		end;
		(sub[S.view] or sub.functions)(d, perf, rec, data, other, names)
		showTools(names)
		if S.view ~= "bench" then
			settingsBar(d, perf, cmp)
		end
		drawRecordings(d, perf, rec, cmp)
	end

	-- A view that needs a switch that is off says so and offers it.
	local function needs(key, what, perf)
		return {
		{
		cells = {what .. " is off: turn it on to record it (click)"},
		colors = {C.warning},
		fonts = {"label"},
		onClick = function()
			perf.set(key, true)
			console.refresh(true)
		end
		}
		}
	end
	local function fnName(name)
		return name or "?"
	end

	-- A ring of 15 trend slots, oldest first.
	local function ordered(ring, last)
		if not ring then
			return nil
		end
		local out = {}
		for i = 1, 15 do
			out[i] = ring[(last + i - 1) % 15 + 1] or 0
		end
		return out
	end

	-- functions: the profiler table, with garbage and a 60-second trend.
	function sub.functions(_, perf, rec, data, other, names)
		layout(true, 2, 0)
		U.clearCanvas()
		local filter = tool("perfFilter", function()
			local f = K.field(toolbar, 160, "Filter", nil)
			f:SetPoint("RIGHT")
			f.onChange = function(value)
				S.filter = value:lower()
				console.refresh(true)
			end
			return f
		end)
		local _ = filter
		names[#names + 1] = "perfFilter"
		tool("perfReset", function()
			local b = K.button(toolbar, "Reset", function()
				local __wl_t30 = U.current() if __wl_t30 ~= nil then __wl_t30 = __wl_t30.runtime.perf end local p = __wl_t30
				if p then
					p.reset()
				end
				console.refresh(true)
			end)
			b:SetPoint("RIGHT", filter, "LEFT", -6, 0)
			return b
		end)
		names[#names + 1] = "perfReset"
		local garbage = data.garbage or (other and other.garbage)
		local rows = {}
		for name, s in pairs(data.functions) do
			if S.filter == "" or name:lower():find(S.filter, 1, true) then
				rows[#rows + 1] = {name = name, s = s}
			end
		end
		local key = S.sort == "avg" and "avg" or S.sort
		table.sort(rows, function(a, b)
			local __wl_t32 = key == "avg" and a.s.total / math.max(a.s.calls, 1) if not __wl_t32 then local __wl_t31 = a.s[key] if __wl_t31 == nil then __wl_t31 = 0 end __wl_t32 = (__wl_t31) end local x = __wl_t32
			local __wl_t34 = key == "avg" and b.s.total / math.max(b.s.calls, 1) if not __wl_t34 then local __wl_t33 = b.s[key] if __wl_t33 == nil then __wl_t33 = 0 end __wl_t34 = (__wl_t33) end local y = __wl_t34
			return x > y
		end)
		local out = {}
		for i, r in ipairs(rows) do
			if i > 300 then
				break
			end
			local s = r.s
			local __wl_t35 = s.over if __wl_t35 == nil then __wl_t35 = 0 end local over = (__wl_t35) > 0
			if other then
				local o = other.functions[r.name]
				local __wl_t37 = K.perMinute(s.total, data) local __wl_t36 = o if __wl_t36 ~= nil then __wl_t36 = __wl_t36.total end local change, tone = K.change(__wl_t37, K.perMinute(__wl_t36, other))
				out[#out + 1] = {
				cells = {r.name, K.count(K.perMinute(s.calls, data)), K.ms(K.perMinute(s.total, data)), K.ms(K.perMinute(s.self, data)), K.ms(s.max), change},
				colors = {C.ink, C.ink, C.ink, C.ink, over and C.warning or C.ink, tone},
				subs = {s.location},
				tall = s.location ~= nil
				}
			else
				out[#out + 1] = {
				cells = {
				r.name, K.count(s.calls), K.ms(s.total), K.ms(s.self), K.ms(s.max), K.ms(s.total / math.max(s.calls, 1)),
				garbage and K.kb(s.selfGarbage) or "", ""
				},
				colors = {C.ink, C.ink, C.ink, C.ink, over and C.warning or C.ink, C.ink, C.ink},
				subs = {s.location},
				tall = s.location ~= nil,
				trend = rec.kind == "live" and ordered(perf.trend[r.name], perf.trendSlot) or nil,
				badge = over and {text = "over budget", color = C.warning} or nil
				}
			end
		end
		if #out == 0 then
			out[1] = {cells = {wl.prof == nil and "No profiled functions" or "No calls in this recording"}, colors = {C.muted}, fonts = {"body"}}
		end
		if other then
			render({
			{title = "Function", width = 0},
			{title = "Calls/min", width = 72, align = "RIGHT", sort = "calls"},
			{title = "ms/min", width = 72, align = "RIGHT", sort = "total"},
			{title = "Self/min", width = 72, align = "RIGHT", sort = "self"},
			{title = "Max", width = 60, align = "RIGHT", sort = "max"},
			{title = "vs " .. other.name, width = 92, align = "RIGHT"}
			}, out)
		else
			render({
			{title = "Function", width = 0},
			{title = "Calls", width = 60, align = "RIGHT", sort = "calls"},
			{title = "Total", width = 60, align = "RIGHT", sort = "total"},
			{title = "Self", width = 60, align = "RIGHT", sort = "self"},
			{title = "Max", width = 56, align = "RIGHT", sort = "max"},
			{title = "Avg", width = 52, align = "RIGHT", sort = "avg"},
			{title = garbage and "Garbage" or "", width = garbage and 64 or 1, align = "RIGHT", sort = garbage and "selfGarbage" or nil},
			{title = rec.kind == "live" and "Last 60 s" or "", width = 80}
			}, out, 8)
		end
	end

	-- The call tree of a recording's paths.
	local function tree(data)
		local top = {kids = {}, order = {}, total = 0}
		for _, p in ipairs(data.paths or {}) do
			local node = top
			for depth, name in ipairs(p.path) do
				local kid = node.kids[name]
				if not kid then
					kid = {name = name, kids = {}, order = {}, calls = 0, total = 0, self = 0, max = 0, depth = depth, parent = node}
					kid.key = (node.key and (node.key .. "\n") or "") .. name
					node.kids[name] = kid
					node.order[#node.order + 1] = kid
				end
				node = kid
			end
			node.calls = node.calls + p.calls
			node.total = node.total + p.total
			node.self = node.self + p.self
			do local __wl_t38 = p.max if __wl_t38 == nil then __wl_t38 = 0 end node.max = math.max(node.max, __wl_t38) end
		end
		-- A root's own time is its handler's (data.roots); its children's
		-- totals add up below it.
		local function sum(node)
			local inner = 0
			for _, kid in ipairs(node.order) do
				inner = inner + sum(kid)
			end
			if node.depth == 1 then
				local r = data.roots[node.name]
				node.total = math.max(r and r.total or 0, inner)
				node.calls = r and r.calls or node.calls
				node.max = r and r.max or node.max
				node.self = node.total - inner
			end
			table.sort(node.order, function(a, b)
				return a.total > b.total
			end)
			return node.total
		end
		top.total = sum(top)
		return top
	end
	local function find(top, key)
		if not key then
			return nil
		end
		local node = top
		for name in (key .. "\n"):gmatch("([^\n]*)\n") do
			node = node.kids[name]
			if not node then
				return nil
			end
		end
		return node
	end

	-- tree: the call tree, rooted at the event, script, timer or command.
	function sub.tree(_, perf, _, data, other, _)
		layout(true, 2, 0)
		U.clearCanvas()
		if not data.callPaths and #(data.paths or {}) == 0 then
			render({{title = "Call tree", width = 0}}, needs("callPaths", "Call paths", perf))
			return
		end
		local top = tree(data)
		local cmpTop = other and tree(other)
		local out = {}
		local function walk(node)
			for _, kid in ipairs(node.order) do
				local open = S.treeOpen[kid.key]
				local has = #kid.order > 0
				local cells = {(has and (open and "- " or "+ ") or "   ") .. kid.name, K.count(kid.calls), K.ms(kid.total), K.ms(kid.self), K.ms(kid.max)}
				local colors = {kid.depth == 1 and C.note or C.ink}
				if other then
					local o = find(cmpTop, kid.key)
					local __wl_t40 = K.perMinute(kid.total, data) local __wl_t39 = o if __wl_t39 ~= nil then __wl_t39 = __wl_t39.total end local change, tone = K.change(__wl_t40, K.perMinute(__wl_t39, other))
					cells[6] = change
					colors[6] = tone
				end
				out[#out + 1] = {
				cells = cells,
				colors = colors,
				indent = (kid.depth - 1) * 16,
				onClick = function()
					S.treeOpen[kid.key] = not open
					console.refresh(true)
				end
				}
				if open then
					walk(kid)
				end
			end
		end
		walk(top)
		if #out == 0 then
			out[1] = {cells = {"No call paths in this recording yet"}, colors = {C.muted}, fonts = {"body"}}
		end
		local columns = {
		{title = "Path", width = 0},
		{title = "Calls", width = 64, align = "RIGHT"},
		{title = "Total", width = 64, align = "RIGHT"},
		{title = "Self", width = 64, align = "RIGHT"},
		{title = "Max", width = 56, align = "RIGHT"}
		}
		if other then
			columns[6] = {title = "vs " .. other.name, width = 92, align = "RIGHT"}
		end
		render(columns, out)
	end

	-- flame: the paths by total time, zoomable.
	function sub.flame(_, perf, _, data, other, _)
		layout(true, 2, 250)
		U.clearCanvas()
		if not data.callPaths and #(data.paths or {}) == 0 then
			U.canvasClick, U.canvasWheel = nil, nil
			render({{title = "Flame graph", width = 0}}, needs("callPaths", "Call paths", perf))
			return
		end
		local top = tree(data)
		local cmpTop = other and tree(other)
		local zoom = find(top, S.flameZoom) or top
		local cw = math.max(plain(U.canvas:GetWidth(), 600) - 8, 100)
		local ch = plain(U.canvas:GetHeight(), 250)

		local spots = {}
		local function draw(node, xpos, w, level)
			if w < 1 or level * 18 > ch - 24 then
				return
			end
			local ypos = ch - 22 - (level + 1) * 18
			local color = level % 2 == 0 and C.accentSoft or C.noteSoft
			if other then
				local o = find(cmpTop, node.key)
				local __wl_t42 = K.perMinute(node.total, data) local __wl_t41 = o if __wl_t41 ~= nil then __wl_t41 = __wl_t41.total end local _, tone = K.change(__wl_t42, K.perMinute(__wl_t41, other))
				color = tone == C.error and C.errorSoft or (tone == C.success and C.successSoft or C.line)
			end
			U.rect(4 + xpos, ypos, w - 1, 18 - 1, color)
			if w > 44 then
				U.label(8 + xpos, ypos + 3, tostring(node.name) .. " " .. tostring(K.ms(node.total)), C.ink, w - 8)
			end
			spots[#spots + 1] = {x = 4 + xpos, y = ypos, w = w, node = node}
			local cx = xpos
			for _, kid in ipairs(node.order) do
				local kw = node.total > 0 and w * kid.total / node.total or 0
				draw(kid, cx, kw, level + 1)
				cx = cx + kw
			end
		end
		if zoom == top then
			local cx = 0
			for _, kid in ipairs(top.order) do
				local kw = top.total > 0 and cw * kid.total / top.total or 0
				draw(kid, cx, kw, 0)
				cx = cx + kw
			end
		else
			draw(zoom, 0, cw, 0)
		end
		U.label(8, ch - 18, zoom == top and "all roots · click a bar to zoom, right-click to zoom out" or (zoom.key:gsub("\n", " > ") .. " · right-click to zoom out"), C.muted, cw - 8)
		U.canvasClick = function(mouse)
			if mouse == "RightButton" then
				S.flameZoom = zoom.parent and zoom.parent.key or nil
				console.refresh(true)
				return
			end
			local cx, cy = U.cursor()
			for _, s in ipairs(spots) do
				if cx >= s.x and cx <= s.x + s.w and cy >= s.y and cy <= s.y + 18 then
					S.flameZoom = s.node.key
					console.refresh(true)
					return
				end
			end
		end
		U.canvasWheel = nil
		-- What the zoomed node consists of.
		local out = {}
		for _, kid in ipairs(zoom.order) do
			out[#out + 1] = {
			cells = {kid.name, K.count(kid.calls), K.ms(kid.total), string.format("%d%%", zoom.total > 0 and math.floor(kid.total / zoom.total * 100) or 0)}
			}
		end
		render({{title = zoom == top and "Root" or "Inside " .. zoom.name, width = 0}, {title = "Calls", width = 64, align = "RIGHT"}, {title = "Total", width = 64, align = "RIGHT"}, {title = "Share", width = 56, align = "RIGHT"}}, out)
	end

	-- by event: time per root, with the function that took most of it.
	function sub.events(_, _, _, data, other, _)
		layout(true, 2, 0)
		U.clearCanvas()
		local rows = {}
		for name, r in pairs(data.roots) do
			rows[#rows + 1] = {name = name, r = r}
		end
		table.sort(rows, function(a, b)
			return a.r.total > b.r.total
		end)
		local out = {}
		for _, x in ipairs(rows) do
			local r = x.r
			local __wl_t44 = r.total > 0 if __wl_t44 then local __wl_t43 = r.topSelf if __wl_t43 == nil then __wl_t43 = 0 end __wl_t44 = math.floor((__wl_t43) / r.total * 100) end local share = __wl_t44 or 0
			local cells = {x.name, r.kind, K.count(r.calls), K.ms(r.total), K.ms(r.max), r.top and fnName(r.top) .. " (" .. share .. "%)" or ""}
			local colors = {C.ink, C.muted, C.ink, C.ink, C.ink, C.muted}
			if other then
				local o = other.roots[x.name]
				local __wl_t46 = K.perMinute(r.total, data) local __wl_t45 = o if __wl_t45 ~= nil then __wl_t45 = __wl_t45.total end local change, tone = K.change(__wl_t46, K.perMinute(__wl_t45, other))
				cells[7], colors[7] = change, tone
			end
			out[#out + 1] = {cells = cells, colors = colors, fonts = {"label", "body", "numeric", "numeric", "numeric", "body", "numeric"}}
		end
		if #out == 0 then
			out[1] = {cells = {"Nothing measured in this recording yet"}, colors = {C.muted}, fonts = {"body"}}
		end
		local columns = {
		{title = "Root", width = 0},
		{title = "Kind", width = 52},
		{title = "Runs", width = 60, align = "RIGHT"},
		{title = "Total", width = 64, align = "RIGHT"},
		{title = "Max", width = 56, align = "RIGHT"},
		{title = "Most of it", width = 180}
		}
		if other then
			columns[7] = {title = "vs " .. other.name, width = 80, align = "RIGHT"}
		end
		render(columns, out)
	end

	-- timeline: time per frame against the budget, garbage, combats and
	-- pulls; a frame selected lists what ran in it.
	function sub.timeline(_, perf, rec, data, other, _)
		layout(true, 2, 170)
		U.clearCanvas()
		local cw = math.max(plain(U.canvas:GetWidth(), 600) - 8, 100)
		local budget = perf.frameBudget
		local slots = math.floor(cw / 3)
		local first, last = 0, -1
		if not rec.saved then
			first, last = perf.frameRange(rec)
		end
		local inRing = last >= first and perf.frameAt(last) ~= nil
		local items = {}
		if inRing then
			local hi = last - S.tlOffset
			local lo = math.max(first, hi - slots + 1)
			for id = lo, hi do
				local t, g, at = perf.frameAt(id)
				if t then
					do local __wl_t47 = g if __wl_t47 == nil then __wl_t47 = 0 end items[#items + 1] = {id = id, time = t, garbage = __wl_t47, at = at} end
				end
			end
		else
			-- Older than the frame ring: one bar per second (its slowest frame).
			local s = data.seconds
			local hi = #s.frames - S.tlOffset
			for i = math.max(1, hi - slots + 1), hi do
				do local __wl_t48 = s.max[i] if __wl_t48 == nil then __wl_t48 = 0 end local __wl_t49 = s.garbage[i] if __wl_t49 == nil then __wl_t49 = 0 end items[#items + 1] = {second = i, time = __wl_t48, garbage = __wl_t49} end
			end
		end
		local most, mostGc = budget * 2, 0
		for _, it in ipairs(items) do
			most = math.max(most, it.time)
			mostGc = math.max(mostGc, it.garbage)
		end

		local base = 44
		for i, it in ipairs(items) do
			local xpos = 4 + (i - 1) * 3
			local color = it.time > budget and C.error or C.lineStrong
			if (S.tlFrame and it.id == S.tlFrame) or (S.tlSecond and it.second == S.tlSecond) then
				color = C.accent
			end
			U.rect(xpos, base, 2, it.time / most * 110, color)
			if mostGc > 0 then
				U.rect(xpos, 14, 2, it.garbage / mostGc * 18, C.noteSoft)
			end
		end
		U.rect(4, base + budget / most * 110, cw, 1, C.warning)
		U.label(cw - 120, base + budget / most * 110 + 2, "budget " .. tostring(K.ms(budget)) .. " ms", C.warning, 120)
		U.label(8, base + 110 + 4, inRing and "frames · " .. tostring(K.ms(most)) .. " ms at the top · wheel to scroll" or "per second (slowest frame) · the frames left the ring", C.muted, cw - 140)
		U.label(8, 2, "garbage", C.note, 80)
		-- Combats and pulls in view.
		if inRing and #items > 0 then
			local lo, hi = items[1].id, items[#items].id
			for _, r in ipairs(perf.recordings) do
				if (r.kind == "combat" or r.kind == "pull") and r.frame then
					local a = math.max(r.frame, lo)
					local __wl_t50 = r.lastFrame if __wl_t50 == nil then __wl_t50 = hi end local b = math.min(__wl_t50, hi)
					if b >= a then
						U.rect(4 + (a - lo) * 3, 36, (b - a + 1) * 3, 4, r.kind == "pull" and C.accent or C.note)
					end
				end
			end
		end
		U.canvasWheel = function(delta)
			S.tlOffset = math.max(0, S.tlOffset + delta * 30)
			console.refresh(true)
		end
		U.canvasClick = function(_)
			local cx = U.cursor()
			local it = items[math.floor((cx - 4) / 3) + 1]
			if it then
				S.tlFrame, S.tlSecond = it.id, it.second
				console.refresh(true)
			end
		end
		local out = {}
		if S.tlFrame and inRing then
			local t, g, _, names, times = perf.frameAt(S.tlFrame)
			if t then
				out[#out + 1] = {cells = {"frame " .. tostring(S.tlFrame), K.ms(t), K.kb(g)}, colors = {C.accent, t > budget and C.error or C.ink, C.ink}}
				for i, name in ipairs(names or {}) do
					out[#out + 1] = {cells = {name, K.ms(times and times[i]), ""}, indent = 16}
				end
			end
		elseif S.tlSecond and not inRing then
			local s, i = data.seconds, S.tlSecond
			out[#out + 1] = {cells = {"second " .. tostring(i), K.ms(s.time[i]), K.kb(s.garbage[i])}, colors = {C.accent}}
			out[#out + 1] = {cells = {tostring(K.count(s.frames[i])) .. " frames, " .. tostring(K.count(s.over[i])) .. " over budget, slowest " .. tostring(K.ms(s.max[i])) .. " ms", "", ""}, indent = 16, fonts = {"body"}}
		end
		local function summary(name, a, b, fmt)
			local cells = {name, fmt(a), ""}
			local colors = {C.muted, C.ink, C.ink}
			if b then
				local change, tone = K.change(a, b)
				cells[3], colors[3] = change, tone
			end
			out[#out + 1] = {cells = cells, colors = colors, fonts = {"body", "numeric", "numeric"}}
		end
		local function avg(d)
			do local __wl_t51 = d.time if __wl_t51 == nil then __wl_t51 = 0 end local __wl_t52 = d.frames if __wl_t52 == nil then __wl_t52 = 0 end return (__wl_t51) / math.max(__wl_t52, 1) end
		end
		summary("time per frame", avg(data), other and avg(other), K.ms)
		summary("frames over budget, per minute", K.perMinute(data.over, data), other and K.perMinute(other.over, other), K.count)
		do local __wl_t53 = data.garbageAll if __wl_t53 == nil then __wl_t53 = 0 end local __wl_t54 = data.duration if __wl_t54 == nil then __wl_t54 = 1 end local __wl_t58 = (__wl_t53) / math.max(__wl_t54, 1) local __wl_t57 = other if __wl_t57 then local __wl_t55 = other.garbageAll if __wl_t55 == nil then __wl_t55 = 0 end local __wl_t56 = other.duration if __wl_t56 == nil then __wl_t56 = 1 end __wl_t57 = (__wl_t55) / math.max(__wl_t56, 1) end summary("garbage per second", __wl_t58, __wl_t57, K.kb) end
		render({{title = "Frame", width = 0}, {title = "Time", width = 80, align = "RIGHT"}, {title = other and ("vs " .. other.name) or "Garbage", width = 92, align = "RIGHT"}}, out)
	end

	-- slow: the slow calls, newest first.
	function sub.slow(_, _, _, data, other, _)
		layout(true, 2, 0)
		U.clearCanvas()
		local out = {}
		if other then
			local change, tone = K.change(K.perMinute(#data.slow, data), K.perMinute(#other.slow, other))
			out[#out + 1] = {cells = {#data.slow .. " slow calls, " .. tostring(K.ms(K.perMinute(#data.slow, data))) .. " per minute", "", "", "", change}, colors = {C.muted, C.ink, C.ink, C.ink, tone}, fonts = {"body"}}
		end
		for _, s in ipairs(data.slow) do
			local open = S.slowOpen == s.stack .. s.at
			out[#out + 1] = {
			cells = {(open and "- " or "+ ") .. s.name, K.ms(s.ms), s.root, s.recording or data.name, string.format("+%.1f s", s.at)},
			subs = {s.location},
			tall = s.location ~= nil,
			colors = {C.ink, C.error, C.muted, C.muted, C.faint},
			fonts = {"label", "numeric", "body", "body", "numeric"},
			onClick = function()
				S.slowOpen = (not open) and (s.stack .. s.at) or nil
				console.refresh(true)
			end,
			onRightClick = function()
				U.openMenu({{label = "Copy", run = function()
					U.copy(tostring(s.name) .. " " .. tostring(K.ms(s.ms)) .. " ms\n" .. tostring(s.stack))
				end}})
			end
			}
			if open then
				for line in tostring(s.stack or ""):gmatch("[^\n]+") do
					out[#out + 1] = {cells = {line}, fonts = {"body"}, indent = 20}
				end
			end
		end
		if #out == 0 then
			out[1] = {cells = {"No calls over " .. tostring(K.ms(data.slowCall)) .. " ms in this recording"}, colors = {C.muted}, fonts = {"body"}}
		end
		render({
		{title = "Slow call", width = 0},
		{title = "ms", width = 60, align = "RIGHT"},
		{title = "Root", width = 150},
		{title = "Recording", width = 90},
		{title = other and ("vs " .. other.name) or "When", width = 70, align = "RIGHT"}
		}, out)
	end

	-- garbage: the rate and memory over time, then per function.
	function sub.garbage(_, perf, _, data, other, _)
		layout(true, 2, 120)
		U.clearCanvas()
		local cw = math.max(plain(U.canvas:GetWidth(), 600) - 8, 100)
		local s = data.seconds
		local n = math.min(#s.garbage, math.floor(cw / 3))
		local most = 0
		for i = #s.garbage - n + 1, #s.garbage do
			do local __wl_t59 = s.garbage[i] if __wl_t59 == nil then __wl_t59 = 0 end most = math.max(most, __wl_t59) end
		end
		for k = 1, n do
			local __wl_t60 = s.garbage[#s.garbage - n + k] if __wl_t60 == nil then __wl_t60 = 0 end local v = __wl_t60
			U.rect(4 + (k - 1) * 3, 18, 2, most > 0 and v / most * 78 or 1, C.noteSoft)
		end
		local mem = data.memory
		local top = 0
		for _, m in ipairs(mem) do
			top = math.max(top, m.kb)
		end
		local __wl_t61 = data.duration if __wl_t61 == nil then __wl_t61 = 1 end local span = math.max(__wl_t61, 1)
		for _, m in ipairs(mem) do
			U.rect(4 + m.at / span * cw, 18 + (top > 0 and m.kb / top * 78 or 0), 3, 3, C.accent)
		end
		U.label(8, 100, "garbage per second (up to " .. tostring(K.kb(most)) .. ") · memory (up to " .. tostring(K.kb(top)) .. ")", C.muted, cw - 8)
		U.label(8, 2, "garbage", C.note, 80)
		U.label(90, 2, "memory", C.accent, 80)
		U.canvasClick, U.canvasWheel = nil, nil
		local out = {}
		local __wl_t62 = data.garbageAll if __wl_t62 == nil then __wl_t62 = 0 end local __wl_t63 = data.duration if __wl_t63 == nil then __wl_t63 = 1 end local rate = (__wl_t62) / math.max(__wl_t63, 1)
		local cells = {"garbage rate (the whole Lua state)", K.kb(rate) .. "/s", "", "", ""}
		local colors = {C.muted, C.ink}
		if other then
			local __wl_t64 = other.garbageAll if __wl_t64 == nil then __wl_t64 = 0 end local __wl_t65 = other.duration if __wl_t65 == nil then __wl_t65 = 1 end local change, tone = K.change(rate, (__wl_t64) / math.max(__wl_t65, 1))
			cells[5], colors[5] = change, tone
		end
		out[#out + 1] = {cells = cells, colors = colors, fonts = {"body"}}
		do local __wl_t67 = not data.garbage if __wl_t67 then local __wl_t66 = data.garbageFns if __wl_t66 == nil then __wl_t66 = 0 end __wl_t67 = (__wl_t66) == 0 end if __wl_t67 then
				for _, r in ipairs(needs("garbage", "Garbage per function", perf)) do
					out[#out + 1] = r
				end
			else
				local rows = {}
				for name, f in pairs(data.functions) do
					do local __wl_t68 = f.garbage if __wl_t68 == nil then __wl_t68 = 0 end if (__wl_t68) > 0 then
							rows[#rows + 1] = {name = name, f = f}
						end end
				end
				table.sort(rows, function(a, b)
					do local __wl_t69 = a.f.selfGarbage if __wl_t69 == nil then __wl_t69 = 0 end local __wl_t70 = b.f.selfGarbage if __wl_t70 == nil then __wl_t70 = 0 end return (__wl_t69) > (__wl_t70) end
				end)
				local __wl_t71 = data.garbageFns if __wl_t71 == nil then __wl_t71 = 0 end local all = math.max(__wl_t71, 0.001)
				for _, r in ipairs(rows) do
					local f = r.f
					local __wl_t73 = K.kb(f.selfGarbage) local __wl_t74 = K.kb(f.garbage) local __wl_t72 = f.garbage if __wl_t72 == nil then __wl_t72 = 0 end local __wl_t76 = K.kb((__wl_t72) / math.max(f.calls, 1)) local __wl_t75 = f.selfGarbage if __wl_t75 == nil then __wl_t75 = 0 end local c = {r.name, __wl_t73, __wl_t74, __wl_t76, string.format("%d%%", math.floor((__wl_t75) / all * 100))}
					local tones = {C.ink, C.ink, C.ink, C.ink, C.muted}
					if other then
						local o = other.functions[r.name]
						do local __wl_t78 = K.perMinute(f.selfGarbage, data) local __wl_t77 = o if __wl_t77 ~= nil then __wl_t77 = __wl_t77.selfGarbage end c[5], tones[5] = K.change(__wl_t78, K.perMinute(__wl_t77, other)) end
					end
					out[#out + 1] = {cells = c, colors = tones}
				end
			end end
		render({
		{title = "Function", width = 0},
		{title = "Own", width = 72, align = "RIGHT"},
		{title = "With callees", width = 92, align = "RIGHT"},
		{title = "Per call", width = 72, align = "RIGHT"},
		{title = other and ("vs " .. other.name) or "Share", width = 80, align = "RIGHT"}
		}, out)
	end

	-- bench: runs of an expression (§7.7.6).
	function sub.bench(d, _, _, _, _, names)
		layout(true, 2, 0)
		U.clearCanvas()
		local expr = tool("benchExpr", function()
			local f = K.field(toolbar2, 360, "Lua expression, run n times", nil)
			f:SetPoint("LEFT")
			return f
		end)
		local count = tool("benchN", function()
			local f = K.field(toolbar2, 72, "n", nil)
			f:SetPoint("LEFT", expr, "RIGHT", 6, 0)
			f:SetText("10000")
			return f
		end)
		tool("benchRun", function()
			local b = K.button(toolbar2, "Run", function()
				local cur = U.current()
				local text = K.plainText(expr:GetText())
				if cur and text ~= "" then
					console.bench(cur, text, tonumber(K.plainText(count:GetText())) or 10000, function()
						console.refresh(true)
					end)
				end
				console.refresh(true)
			end, true)
			b:SetPoint("LEFT", count, "RIGHT", 6, 0)
			return b
		end)
		for _, n in ipairs({"benchExpr", "benchN", "benchRun"}) do
			names[#names + 1] = n
		end
		local out = {}
		for _, run in ipairs(d.benches or {}) do
			local done = run.status == "done"
			out[#out + 1] = {
			cells = {
			run.expr, K.count(done and run.n or run.done),
			done and K.ms(run.min * 1000) or "", done and K.ms(run.median * 1000) or "",
			done and K.ms(run.max * 1000) or "", K.ms(run.total),
			done and "" or run.status
			},
			colors = {C.ink, C.ink, C.ink, C.ink, C.ink, C.ink, run.status:find("^error") and C.error or C.muted},
			fonts = {"label"}
			}
		end
		if #out == 0 then
			out[1] = {cells = {"No runs yet: an expression with ns, modules and globals, as in watches"}, colors = {C.muted}, fonts = {"body"}}
		end
		render({
		{title = "Expression", width = 0},
		{title = "n", width = 64, align = "RIGHT"},
		{title = "Min µs", width = 64, align = "RIGHT"},
		{title = "Median µs", width = 72, align = "RIGHT"},
		{title = "Max µs", width = 64, align = "RIGHT"},
		{title = "Total ms", width = 72, align = "RIGHT"},
		{title = "", width = 90}
		}, out)
	end
end

-- The HUD (§7.7.8): a row per loaded dev build, from their frame rings.
local function buildHud(console, reg)
	local C, plain = K.C, K.plain
	local hud = CreateFrame("Frame", nil, UIParent)
	hud:SetSize(330, 24)
	hud:SetPoint("TOPRIGHT", UIParent, "TOPRIGHT", -220, -40)
	hud:SetFrameStrata("HIGH")
	hud:SetClampedToScreen(true)
	hud:SetMovable(true)
	hud:EnableMouse(true)
	hud:RegisterForDrag("LeftButton")
	hud:Hide()
	local bg = K.fill(hud, C.glass)
	bg:SetAllPoints()
	K.border(hud, C.line)
	local head = K.text(hud, "label", 10, C.faint)
	head:SetPoint("TOPLEFT", 8, -5)
	head:SetText("addon                 frame    avg   garbage  errors")
	local rows = {}
	local state = {wanted = false, cinematic = false}
	local function save()
		local point, _, _, px, py = hud:GetPoint(1)
		for _, n in ipairs(reg.order) do
			devDB(n).hud = {shown = state.wanted, point = K.plainText(point), x = plain(px, 0), y = plain(py, 0)}
		end
	end
	hud:SetScript("OnDragStart", function()
		hud:StartMoving()
	end)
	hud:SetScript("OnDragStop", function()
		hud:StopMovingOrSizing()
		save()
	end)
	local function update()
		local windowOpen = console.window and K.isShown(console.window)
		hud:SetShown(state.wanted and not windowOpen and not state.cinematic)
	end
	local function draw()
		local t = GetTime()
		local y = -20
		local n = 0
		for _, name in ipairs(reg.order) do repeat
				local d = reg.addons[name]
				local perf = d.runtime.perf
				if not perf then
					break
				end
				n = n + 1
				local r = rows[n]
				if not r then
					r = CreateFrame("Button", nil, hud)
					r:SetHeight(18)
					r.flash = K.fill(r, C.errorSoft)
					r.flash:SetAllPoints()
					r.fs = K.text(r, "numeric", 11, C.ink)
					r.fs:SetPoint("LEFT", 8, 0)
					r.fs:SetPoint("RIGHT", -8, 0)
					rows[n] = r
				end
				-- The last frame, the average of the last 60, garbage per second.
				local total, frames = 0, 0
				for id = perf.frame - 60, perf.frame - 1 do
					local ft = perf.frameAt(id)
					if ft then
						total = total + ft
						frames = frames + 1
					end
				end
				local sec = perf.seconds
				local errors = d.runtime.errors and #d.runtime.errors.list or 0
				do local __wl_t80 = name:sub(1, 20) local __wl_t81 = K.ms(perf.lastFrame) local __wl_t82 = K.ms(total / math.max(frames, 1)) local __wl_t79 = sec.garbage[sec.last] if __wl_t79 == nil then __wl_t79 = 0 end r.fs:SetText(string.format("%-20s %6s %6s %8s %6s", __wl_t80, __wl_t81, __wl_t82, K.kb(__wl_t79), errors > 0 and tostring(errors) or "-")) end
				r.fs:SetTextColor(unpack(errors > 0 and C.error or C.ink))
				do local __wl_t83 = perf.lastOver if __wl_t83 == nil then __wl_t83 = -10 end r.flash:SetShown(t - (__wl_t83) < 1) end
				r:SetScript("OnClick", function()
					console.openOn(name, 2)
				end)
				r:ClearAllPoints()
				r:SetPoint("TOPLEFT", 0, y)
				r:SetPoint("TOPRIGHT", 0, y)
				r:Show()
				y = y - 18
			until true end
		for i = n + 1, #rows do
			rows[i]:Hide()
		end
		hud:SetHeight(-y + 4)
	end
	local elapsed = 1
	hud:SetScript("OnUpdate", function(_, dt)
		elapsed = elapsed + (dt or 0)
		if elapsed >= 0.2 then
			elapsed = 0
			draw()
		end
	end)
	-- Hidden during cinematics and movies.
	for _, e in ipairs({"CINEMATIC_START", "CINEMATIC_STOP", "PLAY_MOVIE", "STOP_MOVIE"}) do
		pcall(hud.RegisterEvent, hud, e)
	end
	hud:SetScript("OnEvent", function(_, event)
		state.cinematic = event == "CINEMATIC_START" or event == "PLAY_MOVIE"
		update()
	end)
	for _, n in ipairs(reg.order) do
		local saved = devDB(n).hud
		if type(saved) == "table" then
			state.wanted = saved.shown == true
			if saved.point and saved.point ~= "" then
				hud:ClearAllPoints()
				do local __wl_t84 = saved.x if __wl_t84 == nil then __wl_t84 = 0 end local __wl_t85 = saved.y if __wl_t85 == nil then __wl_t85 = 0 end hud:SetPoint(saved.point, UIParent, saved.point, __wl_t84, __wl_t85) end
			end
			break
		end
	end
	function console.toggleHud()
		state.wanted = not state.wanted
		save()
		update()
	end
	function console.hudShown()
		return state.wanted
	end
	console.updateHud = update
	console.hud = hud
	update()
	return hud
end

-- The error toast (§7.10): new errors while the window is closed; bursts
-- within 2 seconds collapse, it hides after 8 (not while the cursor is on
-- it), waits for the end of combat and makes no sound.
local function buildToast(console)
	local C = K.C
	local toast = CreateFrame("Button", nil, UIParent)
	toast:SetSize(440, 58)
	toast:SetPoint("TOP", UIParent, "TOP", 0, -90)
	toast:SetFrameStrata("DIALOG")
	toast:Hide()
	local bg = K.fill(toast, C.glass)
	bg:SetAllPoints()
	K.border(toast, C.error)
	local bar = K.fill(toast, C.error, "ARTWORK")
	bar:SetWidth(3)
	bar:SetPoint("TOPLEFT")
	bar:SetPoint("BOTTOMLEFT")
	local head = K.text(toast, "label", 12, C.error)
	head:SetPoint("TOPLEFT", 14, -8)
	local msg = K.text(toast, "body", 11, C.ink)
	msg:SetPoint("TOPLEFT", 14, -24)
	msg:SetPoint("TOPRIGHT", -12, -24)
	local action = K.text(toast, "label", 11, C.accent)
	action:SetPoint("BOTTOMRIGHT", -12, 7)
	action:SetText("Open errors →")
	local st = {left = 0, at = -10, count = 0, hover = false, pending = nil}
	local function show()
		local e = st.error
		head:SetText(st.count > 1 and tostring(st.addon) .. " · " .. tostring(st.count) .. " new errors" or tostring(st.addon) .. " · new error")
		msg:SetText((tostring(e.message):match("^[^\n]*")))
		st.left = 8
		toast:Show()
	end
	function console.errorCaptured(addon, e)
		if console.window and K.isShown(console.window) then
			return
		end
		local t = GetTime and GetTime() or 0
		if st.addon == addon and t - st.at < 2 and (K.isShown(toast) or st.pending) then
			st.count = st.count + 1
		else
			st.count = 1
		end
		st.addon, st.error, st.at = addon, e, t
		if InCombatLockdown and InCombatLockdown() then
			st.pending = true
			return
		end
		show()
	end
	function console.hideToast()
		st.pending = nil
		toast:Hide()
	end
	toast:SetScript("OnEnter", function()
		st.hover = true
	end)
	toast:SetScript("OnLeave", function()
		st.hover = false
	end)
	toast:SetScript("OnClick", function()
		toast:Hide()
		console.openOn(st.addon, 5, st.error)
	end)
	toast:SetScript("OnUpdate", function(_, dt)
		if not st.hover then
			st.left = st.left - (dt or 0)
			if st.left <= 0 then
				toast:Hide()
			end
		end
	end)
	pcall(toast.RegisterEvent, toast, "PLAYER_REGEN_ENABLED")
	toast:SetScript("OnEvent", function()
		if st.pending then
			st.pending = nil
			if not (console.window and K.isShown(console.window)) then
				show()
			end
		end
	end)
	console.toast = toast
	return toast
end

-- wowlua runtime: dev console (spec §7.7). This addon's dev data, the
-- shared `WOWLUA_DEV` registry, and — from the newest runtime loaded — the
-- `/wl` command, the dev window, the HUD and the toast.
local dev
dev = {
name = addonName,
info = wl.devInfo or {},
watches = {},
schema = 1,
-- The data the console reads, through one versioned interface.
runtime = wl,
-- A watch from code (`debug.watch`): `fn()` shown live under `label`.
watch = function(label, fn)
	dev.watches[#dev.watches + 1] = {label = label, fn = fn, source = "code"}
end
}
wl.dev = dev
local reg = WOWLUA_DEV
if type(reg) ~= "table" then
	reg = {addons = {}, order = {}}
	WOWLUA_DEV = reg
end
if not reg.addons[addonName] then
	reg.order[#reg.order + 1] = addonName
end
reg.addons[addonName] = dev

local __wl_t86 = wl.devInfo if __wl_t86 ~= nil then __wl_t86 = __wl_t86.runtime end local __wl_t87 = __wl_t86 if __wl_t87 == nil then __wl_t87 = 0 end local VERSION = __wl_t87

-- This runtime's console replaces older ones.
local function install()
	-- Its functions are defined below; the window's are no-ops until it
	-- exists.
	local console = {
	refresh = function() end,
	setTab = function() end,
	saveState = function() end
	}
	reg.version = VERSION
	reg.console = console
	console.buildPerformance = function(U, views, layout)
		buildPerformance(console, U, views, layout)
	end
	console.buildHud = function()
		buildHud(console, reg)
	end

	local function say(text)
		local frame = DEFAULT_CHAT_FRAME
		local line = "|cffffb547wowlua|r " .. text
		if frame then
			frame:AddMessage(line)
		else
			print(line)
		end
	end
	console.say = say
	console.runHints = runHints
	console.newHints = newHints

	-- The addon a command names: exact, then by prefix, case-insensitive; the
	-- only loaded one when none is named.
	function console.find(name)
		if not name or name == "" then
			if #reg.order == 1 then
				return reg.addons[reg.order[1]]
			end
			return nil
		end
		local lower = name:lower()
		for _, n in ipairs(reg.order) do
			if n:lower() == lower then
				return reg.addons[n]
			end
		end
		for _, n in ipairs(reg.order) do
			if n:lower():sub(1, #lower) == lower then
				return reg.addons[n]
			end
		end
		return nil
	end

	local function pad(s, n, right)
		s = tostring(s)
		if #s >= n then
			return s
		end
		local fill = string.rep(" ", n - #s)
		return right and (fill .. s) or (s .. fill)
	end
	local ms = K.ms

	-- Rows of a statistics table, sorted by `key` (descending).
	function console.sorted(stats, key)
		local rows = {}
		for name, s in pairs(stats or {}) do
			rows[#rows + 1] = {name = name, s = s}
		end
		table.sort(rows, function(a, b)
			do local __wl_t88 = a.s[key] if __wl_t88 == nil then __wl_t88 = 0 end local __wl_t89 = b.s[key] if __wl_t89 == nil then __wl_t89 = 0 end return (__wl_t88) > (__wl_t89) end
		end)
		return rows
	end

	local function statTable(stats, sortKey, top, countHead)
		local rows = console.sorted(stats, sortKey)
		say(pad("name", 34) .. pad(countHead, 8, true) .. pad("total", 9, true) .. pad("self", 9, true) .. pad("max", 9, true) .. pad("avg", 9, true))
		for i = 1, math.min(top, #rows) do
			local r = rows[i]
			local s = r.s
			do local __wl_t103 = pad(r.name, 34) local __wl_t90 = s.calls if __wl_t90 == nil then __wl_t90 = s.count end local __wl_t91 = __wl_t90 if __wl_t91 == nil then __wl_t91 = 0 end local __wl_t102 = pad(__wl_t91, 8, true) local __wl_t92 = s.total if __wl_t92 == nil then __wl_t92 = 0 end local __wl_t101 = pad(ms(__wl_t92), 9, true)
				local __wl_t93 = s.self if __wl_t93 == nil then __wl_t93 = s.total end local __wl_t94 = __wl_t93 if __wl_t94 == nil then __wl_t94 = 0 end local __wl_t100 = pad(ms(__wl_t94), 9, true) local __wl_t95 = s.max if __wl_t95 == nil then __wl_t95 = 0 end local __wl_t99 = pad(ms(__wl_t95), 9, true)
				local __wl_t96 = s.total if __wl_t96 == nil then __wl_t96 = 0 end local __wl_t97 = s.calls if __wl_t97 == nil then __wl_t97 = s.count end local __wl_t98 = __wl_t97 if __wl_t98 == nil then __wl_t98 = 1 end say(__wl_t103 .. __wl_t102 .. __wl_t101 .. __wl_t100 .. __wl_t99 .. pad(ms((__wl_t96) / math.max(__wl_t98, 1)), 9, true)) end
		end
		if #rows == 0 then
			say("(no data yet)")
		end
	end

	local function words(msg)
		local out = {}
		for w in (msg or ""):gmatch("%S+") do
			out[#out + 1] = w
		end
		return out
	end

	local commands = {}
	console.commands = commands

	function commands.list()
		for _, n in ipairs(reg.order) do
			local i = reg.addons[n].info
			do local __wl_t104 = i.version if __wl_t104 == nil then __wl_t104 = "?" end local __wl_t105 = i.profile if __wl_t105 == nil then __wl_t105 = "?" end local __wl_t106 = i.build if __wl_t106 == nil then __wl_t106 = "?" end say(tostring(n) .. " " .. tostring(__wl_t104) .. " · " .. tostring(__wl_t105) .. " · build " .. tostring(__wl_t106)) end
		end
	end

	function commands.prof(args)
		local d = console.find(args[1])
		if not d then
			return say("which addon? /wl prof <addon> [top N] [sort calls|total|self|max] [reset]")
		end
		local prof = d.runtime.prof
		if not prof then
			return say(tostring(d.name) .. ": the profiler is off in this build")
		end
		local top, sortKey = 20, "total"
		local i = 2
		while args[i] do
			local w = args[i]:lower()
			if w == "reset" then
				prof.reset()
				return say(tostring(d.name) .. ": profiler statistics reset")
			elseif w == "top" then
				top = tonumber(args[i + 1]) or top
				i = i + 1
			elseif w == "sort" then
				sortKey = (args[i + 1] or "total"):lower()
				i = i + 1
			end
			i = i + 1
		end
		say(tostring(d.name) .. " profiler (ms), by " .. sortKey .. ":")
		statTable(prof.stats, sortKey, top, "calls")
	end

	function commands.mon(args)
		local d = console.find(args[1])
		if not d then
			return say("which addon? /wl mon <addon> [reset]")
		end
		local mon = d.runtime.mon
		if not mon then
			return say(tostring(d.name) .. ": the monitor is off in this build")
		end
		if (args[2] or ""):lower() == "reset" then
			mon.reset()
			return say(tostring(d.name) .. ": monitor reset")
		end
		for _, kind in ipairs({"events", "scripts", "timers", "commands"}) do
			say(tostring(d.name) .. " " .. kind .. ":")
			statTable(mon[kind] or {}, "total", 10, kind == "events" and "count" or "runs")
		end
		local mem = mon.memory[#mon.memory]
		if mem then
			say(string.format("memory %.2f MB", mem / 1024))
		end
	end

	function commands.errors(args)
		local d = console.find(args[1])
		if not d then
			return say("which addon? /wl errors <addon> [clear]")
		end
		local errors = d.runtime.errors
		if not errors then
			return say(tostring(d.name) .. ": error capture is off in this build")
		end
		if (args[2] or ""):lower() == "clear" then
			errors.clear()
			return say(tostring(d.name) .. ": errors cleared")
		end
		if #errors.list == 0 then
			return say(tostring(d.name) .. ": no errors")
		end
		for _, e in ipairs(errors.list) do
			say("|cffff7262" .. tostring(e.message) .. "|r ×" .. tostring(e.count) .. " · " .. tostring(e.time))
		end
	end

	function commands.log(args)
		local d = console.find(args[1])
		if not d then
			return say("which addon? /wl log <addon> [level]")
		end
		local logs = d.runtime.logs
		if not logs then
			return say(tostring(d.name) .. ": this addon does not use wowlua:log")
		end
		if args[2] then
			logs.setLevel(args[2]:lower())
			return say(tostring(d.name) .. ": log level " .. args[2]:lower())
		end
		for _, e in ipairs(logs.history(20)) do
			say(string.format("%.1f", e.time) .. " [" .. tostring(e.level) .. "] " .. tostring(e.message))
		end
	end

	function commands.meta(args)
		local d = console.find(args[1])
		if not d then
			return say("which addon? /wl meta <addon>")
		end
		for k, v in pairs(d.info) do
			say(tostring(d.name) .. " " .. tostring(k) .. " = " .. tostring(v))
		end
		local meta = d.runtime.meta
		for kind, t in pairs(meta or {}) do repeat
				if type(t) ~= "table" then
					break
				end
				for k, v in pairs(t) do
					if type(v) ~= "table" then
						say(tostring(d.name) .. " " .. tostring(kind) .. "." .. tostring(k) .. " = " .. tostring(v))
					end
				end
			until true end
	end

	function commands.inspect(args)
		local d = console.find(args[1])
		if not d or not args[2] then
			return say("/wl inspect <addon> <module>")
		end
		local exports = d.runtime.require(args[2], true)
		local names = {}
		for k in pairs(exports or {}) do
			names[#names + 1] = tostring(k)
		end
		table.sort(names)
		say(d.name .. " " .. args[2] .. ": " .. (#names > 0 and table.concat(names, ", ") or "(no exports)"))
	end

	-- A typed watch: a Lua expression over `ns`, `modules` and globals.
	function console.compile(d, expression)
		local chunk, problem = loadstring("return " .. expression, "=watch")
		if not chunk then
			return nil, problem
		end
		local env = setmetatable({ns = d.runtime.ns, modules = d.runtime.modules}, {__index = _G})
		setfenv(chunk, env)
		return chunk
	end

	function console.show(v)
		if issecretvalue and issecretvalue(v) then
			return "<secret>"
		end
		if type(v) == "string" then
			return string.format("%q", v)
		end
		return tostring(v)
	end

	function commands.watch(args)
		local d = console.find(args[1])
		local expression = table.concat(args, " ", 2)
		if not d or expression == "" then
			return say("/wl watch <addon> <expression>")
		end
		local fn, problem = console.compile(d, expression)
		if not fn then
			return say("|cffff7262" .. tostring(problem) .. "|r")
		end
		d.watches[#d.watches + 1] = {label = expression, fn = fn, source = "typed"}
		local ok, value = pcall(fn)
		say(d.name .. " watch " .. expression .. " = " .. (ok and console.show(value) or ("|cffff7262" .. tostring(value) .. "|r")))
	end

	-- Manual recordings (§7.7.4).
	function commands.record(args)
		local d = console.find(args[1])
		if not d then
			return say("which addon? /wl record <addon> [name] | /wl record <addon> stop")
		end
		local perf = d.runtime.perf
		if not perf then
			return say(tostring(d.name) .. ": this build records no performance data")
		end
		local running = perf.running("manual")
		if (args[2] or ""):lower() == "stop" then
			if not running then
				return say(tostring(d.name) .. ": no recording is running")
			end
			perf.stop(running)
			return say(tostring(d.name) .. ": recording \"" .. tostring(running.name) .. "\" stopped, " .. tostring(K.duration(running.data.duration)))
		end
		if running then
			return say(tostring(d.name) .. ": \"" .. tostring(running.name) .. "\" is recording; /wl record " .. tostring(d.name) .. " stop")
		end
		local rec = perf.start("manual", args[2] and table.concat(args, " ", 2) or nil)
		say(tostring(d.name) .. ": recording \"" .. tostring(rec.name) .. "\"; /wl record " .. tostring(d.name) .. " stop")
	end

	-- Bench (§7.7.6): an expression n times, in chunks of at most 50 ms per
	-- frame, never in combat; the last 20 runs per addon.
	function console.bench(d, expression, n, onDone)
		if InCombatLockdown and InCombatLockdown() then
			say("bench does not run in combat")
			return nil
		end
		if console.benchRun then
			say("a bench is running")
			return nil
		end
		local fn, problem = console.compile(d, expression)
		d.benches = d.benches or {}
		local run = {expr = expression, n = math.max(math.floor(n), 1), done = 0, total = 0, times = {}, status = "running"}
		table.insert(d.benches, 1, run)
		while #d.benches > 20 do
			table.remove(d.benches)
		end
		if not fn then
			run.status = "error: " .. tostring(problem)
			return run
		end
		local clock = debugprofilestop
		local function stop(status)
			run.status = status
			console.benchRun = nil
			if console.benchFrame then
				console.benchFrame:SetScript("OnUpdate", nil)
			end
			if status == "done" then
				table.sort(run.times)
				local t = run.times
				run.min, run.max = t[1] / 1000, t[#t] / 1000
				local mid = math.floor((#t + 1) / 2)
				run.median = (#t % 2 == 1 and t[mid] or (t[mid] + t[mid + 1]) / 2) / 1000
			end
			run.times = nil
			if onDone then
				onDone(run)
			end
		end
		local function chunk()
			if InCombatLockdown and InCombatLockdown() then
				return stop("stopped: combat")
			end
			local start = clock()
			while run.done < run.n and clock() - start < 50 do
				local t0 = clock()
				local ok, e = pcall(fn)
				local dt = clock() - t0
				if not ok then
					return stop("error: " .. tostring(e))
				end
				run.done = run.done + 1
				run.times[run.done] = dt * 1000
				run.total = run.total + dt
			end
			if run.done >= run.n then
				stop("done")
			end
		end
		console.benchRun = run
		if type(CreateFrame) == "function" then
			console.benchFrame = console.benchFrame or CreateFrame("Frame")
			console.benchFrame:SetScript("OnUpdate", chunk)
		end
		chunk()
		return run
	end

	function commands.bench(args)
		local d = console.find(args[1])
		if not d or not args[2] then
			return say("/wl bench <addon> <expression> [n]")
		end
		-- A last number is n when the rest is an expression (`1 + 1` is not
		-- `1 +` run once).
		local n = 10000
		local expression = table.concat(args, " ", 2)
		local count = tonumber(args[#args])
		if #args > 2 and count then
			local rest = table.concat(args, " ", 2, #args - 1)
			if console.compile(d, rest) then
				n, expression = count, rest
			end
		end
		console.bench(d, expression, n, function(run)
			if run.status == "done" then
				say(string.format("%s bench %s ×%d: min %s µs, median %s µs, max %s µs, total %s ms", d.name, run.expr, run.n,
				ms(run.min * 1000), ms(run.median * 1000), ms(run.max * 1000), ms(run.total)))
			else
				say(tostring(d.name) .. " bench " .. tostring(run.expr) .. ": " .. tostring(run.status))
			end
			console.refresh(true)
		end)
	end

	-- Performance hints (§7.7.7).
	function commands.hints(args)
		local d = console.find(args[1])
		if not d then
			return say("which addon? /wl hints <addon>")
		end
		if not d.runtime.perf then
			return say(tostring(d.name) .. ": this build records no performance data")
		end
		local list = runHints(d)
		if #list == 0 then
			return say(tostring(d.name) .. ": no hints for the live recording")
		end
		for _, x in ipairs(list) do
			say("|cffecd660hint[" .. tostring(x.code) .. "]|r " .. tostring(x.subject) .. ": " .. tostring(x.evidence) .. ". " .. tostring(x.fix) .. ".")
		end
	end

	function commands.hud()
		if not console.hud and type(CreateFrame) == "function" then
			buildHud(console, reg)
		end
		if console.toggleHud then
			console.toggleHud()
		end
	end

	function commands.reload()
		ReloadUI()
	end

	function console.command(msg)
		local args = words(msg)
		local name = table.remove(args, 1)
		if not name then
			if console.toggle then
				return console.toggle()
			end
			return commands.list()
		end
		local f = commands[name:lower()]
		if f then
			return f(args)
		end
		say("/wl [list | prof | mon | errors | log | meta | inspect | watch | record | bench | hints | hud | reload]")
	end

	if type(SlashCmdList) == "table" then
		_G.SLASH_WOWLUA1 = "/wl"
		SlashCmdList.WOWLUA = function(msg)
			reg.console.command(msg)
		end
	end
	-- `/wl`: the dev window, built when first opened; the HUD and the toast.
	if type(CreateFrame) == "function" then
		local function window()
			if not console.window then
				console.window = buildDevWindow(console, reg)
			end
			return console.window
		end
		function console.toggle()
			local w = window()
			if K.isShown(w) then
				w:Hide()
			else
				w:Show()
				console.refresh()
			end
		end
		-- The HUD once every addon's saved data is loaded (at login), for
		-- the console that serves then; the toast at the first error.
		-- (Quietly: a missing HUD must not stop the addon; `/wl hud` builds
		-- it again and says why.)
		local function hud()
			if reg.console == console and not console.hud then
				pcall(buildHud, console, reg)
			end
		end
		if IsLoggedIn and IsLoggedIn() then
			hud()
		else
			local login = CreateFrame("Frame")
			if pcall(login.RegisterEvent, login, "PLAYER_LOGIN") then
				login:SetScript("OnEvent", hud)
			end
		end
		console.errorCaptured = function(addon, e)
			buildToast(console)
			console.errorCaptured(addon, e)
		end
		-- Until the window exists, opening goes through it.
		console.openOn = function(name, tab, e)
			window()
			console.openOn(name, tab, e)
		end
	end
	dev.console = console
end
do local __wl_t107 = reg.version if __wl_t107 == nil then __wl_t107 = -1 end if (__wl_t107) < VERSION then
		install()
	end end
