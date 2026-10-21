-- wowlua runtime: the dev window (spec §7.7.1), drawn by the newest
-- console loaded. Its look follows the SuuUI design system: neutral
-- surfaces, gilt hairlines, `arcane` for what is chosen, `amber` and
-- `ember` only for warnings and errors.
local function buildDevWindow(console, reg)
	local C = {
	glass = {0.075, 0.067, 0.086, 0.94},
	sunk = {0.082, 0.071, 0.059, 1},
	raised = {0.110, 0.094, 0.082, 1},
	surface = {0.075, 0.067, 0.086, 1},
	gilt = {0.788, 0.655, 0.353, 1},
	giltText = {0.902, 0.800, 0.541, 1},
	giltLine = {0.788, 0.655, 0.353, 0.38},
	giltFaint = {0.788, 0.655, 0.353, 0.14},
	ink = {0.945, 0.918, 0.859, 1},
	inkMuted = {0.639, 0.608, 0.545, 1},
	inkFaint = {0.478, 0.447, 0.392, 1},
	arcane = {0.247, 0.816, 1, 1},
	arcaneSoft = {0.059, 0.102, 0.122, 1},
	emberText = {1, 0.604, 0.502, 1},
	amber = {1, 0.698, 0.247, 1},
	onSignal = {0.071, 0.055, 0.043, 1},
	progress = {0.549, 0.482, 0.369, 1}
	}
	local DIR = "Interface\\AddOns\\" .. tostring(addonName) .. "\\wowlua\\fonts\\"
	local FONTS = {
	display = DIR .. "MarcellusSC-Regular.ttf",
	body = DIR .. "Figtree-Regular.ttf",
	label = DIR .. "Figtree-SemiBold.ttf",
	numeric = DIR .. "Figtree-Bold.ttf"
	}
	local __wl_t1 = STANDARD_TEXT_FONT if __wl_t1 == nil then __wl_t1 = "Fonts\\FRIZQT__.TTF" end local FALLBACK = __wl_t1





	local function text(parent, kind, size, color)
		local fs = parent:CreateFontString(nil, "OVERLAY")
		if not fs:SetFont(FONTS[kind] or FONTS.body, size, "") then
			fs:SetFont(FALLBACK, size, "")
		end
		fs:SetTextColor(unpack(color or C.ink))
		fs:SetJustifyH("LEFT")
		fs:SetWordWrap(false)
		return fs
	end
	local function fill(parent, color, layer)
		local t = parent:CreateTexture(nil, layer or "BACKGROUND")
		t:SetColorTexture(unpack(color))
		return t
	end
	-- A 1px border inside `frame`.
	local function border(frame, color)
		local edges = {}
		for i = 1, 4 do
			edges[i] = fill(frame, color, "BORDER")
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
	local function hairline(parent, color)
		local t = fill(parent, color or C.giltFaint, "ARTWORK")
		t:SetHeight(1)
		return t
	end
	-- The Jewel: a small rotated square.
	local function diamond(parent, size, color)
		local t = fill(parent, color or C.gilt, "OVERLAY")
		t:SetSize(size, size)
		if t.SetRotation then
			t:SetRotation(math.rad(45))
		end
		return t
	end
	-- A quiet button (or `primary`, the one action of a view).
	local function button(parent, label, onClick, primary)
		local b = CreateFrame("Button", nil, parent)
		b:SetHeight(26)
		local bg = fill(b, primary and C.arcane or C.raised)
		bg:SetAllPoints()
		local edges = border(b, primary and C.arcane or C.giltLine)
		local fs = text(b, "label", 12, primary and C.onSignal or C.ink)
		fs:SetPoint("CENTER")
		fs:SetText(label)
		b:SetWidth(math.max(64, (fs:GetStringWidth() or 40) + 24))
		b:SetScript("OnEnter", function()
			if not primary then
				for _, e in ipairs(edges) do
					e:SetColorTexture(unpack(C.gilt))
				end
			end
		end)
		b:SetScript("OnLeave", function()
			for _, e in ipairs(edges) do
				e:SetColorTexture(unpack(primary and C.arcane or C.giltLine))
			end
		end)
		b:SetScript("OnClick", onClick)
		b.label = fs
		return b
	end
	-- A field on `surface-sunk`.
	local function field(parent, width, placeholder, onEnter)
		local box = CreateFrame("EditBox", nil, parent)
		box:SetSize(width, 26)
		box:SetAutoFocus(false)
		local bg = fill(box, C.sunk)
		bg:SetAllPoints()
		border(box, C.giltLine)
		if not box:SetFont(FONTS.body, 12, "") then
			box:SetFont(FALLBACK, 12, "")
		end
		box:SetTextColor(unpack(C.ink))
		box:SetTextInsets(8, 8, 0, 0)
		local hint = text(box, "body", 12, C.inkMuted)
		hint:SetPoint("LEFT", 8, 0)
		hint:SetText(placeholder)
		box:SetScript("OnTextChanged", function(self)
			hint:SetShown(self:GetText() == "")
			if self.onChange then
				self.onChange(self:GetText())
			end
		end)
		box:SetScript("OnEscapePressed", box.ClearFocus)
		box:SetScript("OnEnterPressed", function(self)
			if onEnter then
				onEnter(self:GetText())
			end
			self:ClearFocus()
		end)
		return box
	end
	-- Two to five named options; the chosen one in `arcane`.
	local function segmented(parent, options, onPick)
		local f = CreateFrame("Frame", nil, parent)
		f:SetHeight(26)
		local bg = fill(f, C.sunk)
		bg:SetAllPoints()
		border(f, C.giltLine)
		local x = 1
		f.buttons = {}
		for _, opt in ipairs(options) do
			local b = CreateFrame("Button", nil, f)
			local fs = text(b, "label", 12, C.inkMuted)
			fs:SetPoint("CENTER")
			fs:SetText(opt.label)
			local w = math.max(56, (fs:GetStringWidth() or 40) + 20)
			b:SetSize(w, 24)
			b:SetPoint("TOPLEFT", x, -1)
			x = x + w
			local on = fill(b, C.arcaneSoft)
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
				b.fs:SetTextColor(unpack(chosen and C.arcane or C.inkMuted))
			end
		end
		return f
	end

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
		win:SetResizeBounds(720, 480)
	end
	local ground = fill(win, C.glass)
	ground:SetAllPoints()
	border(win, C.giltLine)
	if UISpecialFrames then
		table.insert(UISpecialFrames, "WowluaDevWindow")
	end

	local state = {tab = 2, sort = "total", monitor = "events", level = "all", filter = "", expanded = {}}

	-- Title bar: the Jewel, the title, the key chip, close.
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
	local jewel = diamond(titleBar, 7)
	jewel:SetPoint("TOPLEFT", win, "TOPLEFT", -3, 3)
	local title = text(titleBar, "display", 13, C.giltText)
	title:SetPoint("LEFT", 14, 0)
	title:SetText("wowlua dev")
	local chip = text(titleBar, "numeric", 10, C.inkMuted)
	chip:SetPoint("LEFT", title, "RIGHT", 10, 0)
	chip:SetText("/wl")
	local close = CreateFrame("Button", nil, titleBar)
	close:SetSize(24, 24)
	close:SetPoint("RIGHT", -8, 0)
	local x = text(close, "label", 14, C.inkMuted)
	x:SetPoint("CENTER")
	x:SetText("×")
	close:SetScript("OnClick", function()
		win:Hide()
	end)

	-- Addon selector: a dropdown field (SuuUI `Select`) with the loaded
	-- dev builds.
	local addonButton = CreateFrame("Button", nil, titleBar)
	addonButton:SetSize(200, 26)
	addonButton:SetPoint("RIGHT", close, "LEFT", -10, 0)
	local addonBg = fill(addonButton, C.sunk)
	addonBg:SetAllPoints()
	local addonEdges = border(addonButton, C.giltLine)
	local addonLabel = text(addonButton, "label", 12, C.ink)
	addonLabel:SetPoint("LEFT", 10, 0)
	addonLabel:SetPoint("RIGHT", -26, 0)
	-- The chevron: two short rotated strokes.
	local chevron = {}
	for i, angle in ipairs({-45, 45}) do
		local stroke = fill(addonButton, C.inkMuted, "OVERLAY")
		stroke:SetSize(6, 1.5)
		if stroke.SetRotation then
			stroke:SetRotation(math.rad(angle))
		end
		stroke:SetPoint("RIGHT", addonButton, "RIGHT", i == 1 and -15 or -11, 0)
		chevron[i] = stroke
	end
	local selected = reg.order[1]
	local function current()
		return reg.addons[selected] or reg.addons[reg.order[1]]
	end
	-- The open list, directly under the field.
	local addonList = CreateFrame("Frame", nil, win)
	addonList:SetFrameStrata("FULLSCREEN_DIALOG")
	addonList:SetPoint("TOPRIGHT", addonButton, "BOTTOMRIGHT", 0, -2)
	addonList:Hide()
	local listBg = fill(addonList, C.surface)
	listBg:SetAllPoints()
	border(addonList, C.giltLine)
	local addonRows = {}
	local function openAddonList()
		local y = -4
		for i, name in ipairs(reg.order) do
			local b = addonRows[i]
			if not b then
				b = CreateFrame("Button", nil, addonList)
				b:SetHeight(24)
				b.hover = fill(b, C.raised)
				b.hover:SetAllPoints()
				b.hover:Hide()
				b.mark = diamond(b, 5, C.arcane)
				b.mark:SetPoint("LEFT", 10, 0)
				b.fs = text(b, "label", 12, C.ink)
				b.fs:SetPoint("LEFT", 24, 0)
				b.info = text(b, "body", 11, C.inkMuted)
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
			b.fs:SetTextColor(unpack(chosen and C.ink or C.inkMuted))
			b.mark:SetShown(chosen)
			local entry = reg.addons[name]
			do local __wl_t2 = entry if __wl_t2 ~= nil then __wl_t2 = __wl_t2.info.version end local __wl_t3 = __wl_t2 if __wl_t3 == nil then __wl_t3 = "" end b.info:SetText(__wl_t3) end
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
		addonList:SetSize(math.max(addonButton:GetWidth() or 200, 240), -y + 4)
		addonList:Show()
	end
	addonButton:SetScript("OnClick", function()
		if addonList:IsShown() then
			addonList:Hide()
		else
			openAddonList()
		end
	end)
	addonButton:SetScript("OnEnter", function()
		for _, e in ipairs(addonEdges) do
			e:SetColorTexture(unpack(C.gilt))
		end
	end)
	addonButton:SetScript("OnLeave", function()
		for _, e in ipairs(addonEdges) do
			e:SetColorTexture(unpack(C.giltLine))
		end
	end)

	-- Tabs.
	local TABS = {"Overview", "Profiler", "Monitor", "Log", "Errors", "Watch", "Inspector"}
	local tabBar = CreateFrame("Frame", nil, win)
	tabBar:SetPoint("TOPLEFT", 14, -40)
	tabBar:SetPoint("TOPRIGHT", -14, -40)
	tabBar:SetHeight(26)
	local tabRule = hairline(tabBar, C.giltFaint)
	tabRule:SetPoint("BOTTOMLEFT")
	tabRule:SetPoint("BOTTOMRIGHT")
	local tabButtons = {}
	local tx = 0
	for i, name in ipairs(TABS) do
		local b = CreateFrame("Button", nil, tabBar)
		local fs = text(b, "display", 13, C.inkMuted)
		fs:SetPoint("LEFT")
		fs:SetText(name)
		local w = (fs:GetStringWidth() or 60) + 4
		b:SetSize(w, 26)
		b:SetPoint("LEFT", tx, 0)
		tx = tx + (w + 12)
		local line = hairline(b, C.gilt)
		line:SetPoint("BOTTOMLEFT")
		line:SetPoint("BOTTOMRIGHT")
		local gem = diamond(b, 5)
		gem:SetPoint("BOTTOM", 0, -2)
		b.fs, b.line, b.gem = fs, line, gem
		b:SetScript("OnClick", function()
			state.tab = i
			console.saveState()
			console.refresh(true)
		end)
		tabButtons[i] = b
	end

	-- Body: a toolbar row, then a scrolling list.
	local body = CreateFrame("Frame", nil, win)
	body:SetPoint("TOPLEFT", 14, -76)
	body:SetPoint("BOTTOMRIGHT", -14, 48)
	local toolbar = CreateFrame("Frame", nil, body)
	toolbar:SetPoint("TOPLEFT")
	toolbar:SetPoint("TOPRIGHT")
	toolbar:SetHeight(28)
	local scroll = CreateFrame("ScrollFrame", nil, body)
	scroll:SetPoint("TOPLEFT", 0, -36)
	scroll:SetPoint("BOTTOMRIGHT", -6, 0)
	local list = CreateFrame("Frame", nil, scroll)
	list:SetSize(880 - 40, 10)
	scroll:SetScrollChild(list)
	scroll:EnableMouseWheel(true)
	local thumb = fill(body, C.giltLine, "OVERLAY")
	thumb:SetWidth(3)
	local function layoutThumb()
		local __wl_t4 = scroll:GetHeight() if __wl_t4 == nil then __wl_t4 = 1 end local __wl_t5 = list:GetHeight() if __wl_t5 == nil then __wl_t5 = 1 end local view, total = __wl_t4, __wl_t5
		if total <= view + 1 then
			thumb:Hide()
			return
		end
		thumb:Show()
		local h = math.max(20, view * view / total)
		local __wl_t6 = scroll:GetVerticalScroll() if __wl_t6 == nil then __wl_t6 = 0 end local offset = (__wl_t6) / (total - view) * (view - h)
		thumb:ClearAllPoints()
		thumb:SetPoint("TOPRIGHT", scroll, "TOPRIGHT", 6, -offset)
		thumb:SetHeight(h)
	end
	scroll:SetScript("OnMouseWheel", function(self, delta)
		local __wl_t7 = list:GetHeight() if __wl_t7 == nil then __wl_t7 = 0 end local __wl_t8 = self:GetHeight() if __wl_t8 == nil then __wl_t8 = 0 end local max = math.max(0, (__wl_t7) - (__wl_t8))
		do local __wl_t9 = self:GetVerticalScroll() if __wl_t9 == nil then __wl_t9 = 0 end self:SetVerticalScroll(math.min(max, math.max(0, (__wl_t9) - delta * 40))) end
		layoutThumb()
	end)

	-- Footer.
	local footer = CreateFrame("Frame", nil, win)
	footer:SetPoint("BOTTOMLEFT", 14, 10)
	footer:SetPoint("BOTTOMRIGHT", -14, 10)
	footer:SetHeight(28)
	local footRule = hairline(footer, C.giltFaint)
	footRule:SetPoint("TOPLEFT", 0, 4)
	footRule:SetPoint("TOPRIGHT", 0, 4)
	local footText = text(footer, "body", 11, C.inkMuted)
	footText:SetPoint("LEFT")
	local reload = button(footer, "Reload UI", function()
		ReloadUI()
	end)
	reload:SetPoint("RIGHT")

	-- Resize grip.
	local grip = CreateFrame("Button", nil, win)
	grip:SetSize(14, 14)
	grip:SetPoint("BOTTOMRIGHT", -2, 2)
	local gripGem = diamond(grip, 5, C.giltLine)
	gripGem:SetPoint("CENTER")
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
	menu:SetSize(180, 10)
	menu:Hide()
	local menuBg = fill(menu, C.surface)
	menuBg:SetAllPoints()
	border(menu, C.giltLine)
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
				b:SetSize(178, 24)
				local hover = fill(b, C.raised)
				hover:SetAllPoints()
				hover:Hide()
				b.fs = text(b, "label", 12, C.ink)
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
		local scale = UIParent:GetEffectiveScale()
		menu:ClearAllPoints()
		menu:SetPoint("TOPLEFT", UIParent, "BOTTOMLEFT", cx / scale, cy / scale)
		menu:Show()
	end

	-- A box to copy a value from (the client has no clipboard API).
	local copyBox = field(win, 420, "", nil)
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

	-- Rows: a pool of frames with cells at column offsets.
	local rows = {}
	local function row(i)
		local r = rows[i]
		if not r then
			r = CreateFrame("Button", nil, list)
			r:RegisterForClicks("LeftButtonUp", "RightButtonUp")
			r.hover = fill(r, C.raised)
			r.hover:SetAllPoints()
			r.hover:Hide()
			r.rule = hairline(r, C.giltFaint)
			r.rule:SetPoint("BOTTOMLEFT")
			r.rule:SetPoint("BOTTOMRIGHT")
			r.cells, r.subs = {}, {}
			r.badge = CreateFrame("Frame", nil, r)
			r.badge:SetSize(40, 15)
			r.badge.bg = fill(r.badge, C.amber)
			r.badge.bg:SetAllPoints()
			r.badge.fs = text(r.badge, "display", 10.5, C.onSignal)
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
			fs = text(r, "numeric", 12, C.ink)
			r.cells[j] = fs
		end
		return fs
	end
	local function sub(r, j)
		local fs = r.subs[j]
		if not fs then
			fs = text(r, "body", 11, C.inkMuted)
			r.subs[j] = fs
		end
		return fs
	end
	local width = 880 - 40
	-- columns: { title, width (0 = rest), align, sort key }; data rows:
	-- { cells = {}, subs = {}, colors = {}, fonts = {}, badge, indent,
	--   tall, onClick, onRightClick }.
	local function render(columns, data)
		width = scroll:GetWidth() or (880 - 40)
		list:SetWidth(width)
		local fixed = 0
		for _, c in ipairs(columns) do
			do local __wl_t10 = c.width if __wl_t10 == nil then __wl_t10 = 0 end fixed = fixed + (__wl_t10) + 12 end
		end
		local y = 0
		local function x0(j)
			local xpos = 8
			for k = 1, j - 1 do
				local w = columns[k].width
				xpos = xpos + ((w and w > 0) and w or (width - fixed - 16)) + 12
			end
			return xpos
		end
		local function colWidth(j)
			local w = columns[j].width
			return (w and w > 0) and w or (width - fixed - 16)
		end
		-- Header row.
		local hr = row(1)
		hr:SetPoint("TOPLEFT", 0, 0)
		hr:SetSize(width, 24)
		hr.onClick, hr.onRightClick = nil, nil
		hr.badge:Hide()
		hr.rule:SetColorTexture(unpack(C.giltLine))
		for j, c in ipairs(columns) do
			local fs = cell(hr, j)
			if not fs:SetFont(FONTS.body, 11, "") then
				fs:SetFont(FALLBACK, 11, "")
			end
			local active = c.sort and c.sort == state.sort
			fs:SetTextColor(unpack(active and C.arcane or C.inkMuted))
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
		for j = 1, #hr.subs do
			hr.subs[j]:Hide()
		end
		hr.onClick = function()
			-- Sorting: the column under the cursor.
			local __wl_t12 = GetCursorPosition() / UIParent:GetEffectiveScale() local __wl_t11 = hr:GetLeft() if __wl_t11 == nil then __wl_t11 = 0 end local cx = __wl_t12 - (__wl_t11)
			for j, c in ipairs(columns) do
				if c.sort and cx >= x0(j) and cx <= x0(j) + colWidth(j) then
					state.sort = c.sort
					console.refresh(true)
				end
			end
		end
		hr:Show()
		y = 24
		for i, d in ipairs(data) do
			local r = row(i + 1)
			local h = d.tall and 34 or 24
			r:ClearAllPoints()
			r:SetPoint("TOPLEFT", 0, -y)
			r:SetSize(width, h)
			r.rule:SetColorTexture(unpack(C.giltFaint))
			r.onClick, r.onRightClick = d.onClick, d.onRightClick
			for j = 1, #columns do
				local fs = cell(r, j)
				local __wl_t13 = d.fonts if __wl_t13 ~= nil then __wl_t13 = __wl_t13[j] end local __wl_t14 = __wl_t13 if __wl_t14 == nil then __wl_t14 = (j == 1 and "label" or "numeric") end local font = __wl_t14
				if not fs:SetFont(FONTS[font], 12, "") then
					fs:SetFont(FALLBACK, 12, "")
				end
				do local __wl_t15 = d.colors if __wl_t15 ~= nil then __wl_t15 = __wl_t15[j] end local __wl_t16 = __wl_t15 if __wl_t16 == nil then __wl_t16 = C.ink end fs:SetTextColor(unpack(__wl_t16)) end
				fs:SetText(d.cells[j] or "")
				fs:ClearAllPoints()
				local indent = (j == 1 and d.indent) or 0
				fs:SetPoint("TOPLEFT", r, "TOPLEFT", x0(j) + indent, d.tall and -4 or -5)
				fs:SetWidth(colWidth(j) - indent)
				fs:SetJustifyH(columns[j].align or "LEFT")
				fs:Show()
				local s = sub(r, j)
				if d.subs and d.subs[j] then
					s:SetText(d.subs[j])
					s:ClearAllPoints()
					s:SetPoint("TOPLEFT", fs, "BOTTOMLEFT", 0, -2)
					s:SetWidth(colWidth(j) - indent)
					s:Show()
				else
					s:Hide()
				end
			end
			for j = #columns + 1, #r.cells do
				r.cells[j]:Hide()
			end
			if d.badge then
				r.badge.fs:SetText(d.badge.text)
				r.badge.bg:SetColorTexture(unpack(d.badge.color or C.amber))
				r.badge:SetWidth((r.badge.fs:GetStringWidth() or 30) + 10)
				r.badge:ClearAllPoints()
				r.badge:SetPoint("RIGHT", r, "RIGHT", -8, 0)
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

	local function ms(v)
		return string.format((v or 0) >= 10 and "%.1f" or "%.2f", v or 0)
	end
	local function count(n)
		local __wl_t17 = n if __wl_t17 == nil then __wl_t17 = 0 end local s = tostring(math.floor(__wl_t17))
		local out = s:reverse():gsub("(%d%d%d)", "%1,"):reverse()
		return (out:gsub("^,", ""))
	end

	-- Inspector: the value shown and how to reach it again.
	local inspected = {label = "ns", get = function()
		local d = current()
		do local __wl_t18 = d if __wl_t18 ~= nil then __wl_t18 = __wl_t18.runtime.ns end return __wl_t18 end
	end}
	local function inspect(label, get)
		inspected = {label = label, get = get}
		state.expanded = {[""] = true}
		state.tab = 7
		console.refresh(true)
	end
	local function addWatch(label, fn, source)
		local d = current()
		if d then
			d.watches[#d.watches + 1] = {label = label, fn = fn, source = source}
		end
		state.tab = 6
		console.refresh(true)
	end

	local views = {}
	function views.overview(d)
		showTools({})
		local data = {}
		for _, k in ipairs({"version", "profile", "build", "flavors", "compiler"}) do
			data[#data + 1] = {cells = {k, tostring(d.info[k] or "?")}, colors = {C.inkMuted, C.ink}}
		end
		local rt = d.runtime
		local toolRows = {
		{"Profiler", rt.prof and "on" or "off"},
		{"Monitor", rt.mon and "events, scripts, timers, memory" or "off"},
		{"Error capture", rt.errors and "on" or "off"},
		{"Log", rt.logs and "wowlua:log" or "not used"}
		}
		for _, t in ipairs(toolRows) do
			data[#data + 1] = {cells = {t[1], t[2]}, colors = {C.ink, C.inkMuted}, fonts = {"label", "body"}}
		end
		local __wl_t19 = rt.mon if __wl_t19 ~= nil then __wl_t19 = __wl_t19.memory end local __wl_t20 = __wl_t19 if __wl_t20 == nil then __wl_t20 = {} end local mem = __wl_t20
		local last = mem[#mem]
		data[#data + 1] = {
		cells = {"Memory", last and string.format("%.2f MB · %d samples", last / 1024, #mem) or "no samples yet"},
		colors = {C.ink, C.ink}
		}
		render({{title = "Build", width = 200}, {title = "", width = 0}}, data)
	end
	function views.profiler(d)
		local filter = tool("filter", function()
			local f = field(toolbar, 260, "Filter functions")
			f:SetPoint("LEFT")
			f.onChange = function(value)
				state.filter = value:lower()
				console.refresh(true)
			end
			return f
		end)
		local reset = tool("profReset", function()
			local b = button(toolbar, "Reset", function()
				local cur = current()
				if cur and cur.runtime.prof then
					cur.runtime.prof.reset()
				end
				console.refresh(true)
			end)
			b:SetPoint("RIGHT")
			return b
		end)
		showTools({"filter", "profReset"})
		local _ = filter, reset
		local prof = d.runtime.prof
		local data = {}
		do local __wl_t21 = prof if __wl_t21 ~= nil then __wl_t21 = __wl_t21.stats end local __wl_t22 = __wl_t21 if __wl_t22 == nil then __wl_t22 = {} end for _, r in ipairs(console.sorted(__wl_t22, state.sort)) do repeat
					if not (state.filter == "" or r.name:lower():find(state.filter, 1, true)) then
						break
					end
					local s = r.s
					local __wl_t23 = s.max if __wl_t23 == nil then __wl_t23 = 0 end local slow = (__wl_t23) > 16.7
					data[#data + 1] = {
					cells = {r.name, count(s.calls), ms(s.total), ms(s.self), ms(s.max), ms(s.total / math.max(s.calls, 1))},
					subs = {s.location},
					colors = {C.ink, C.ink, C.ink, C.ink, slow and C.amber or C.ink, C.ink},
					badge = slow and {text = "Slow", color = C.amber} or nil,
					tall = s.location ~= nil
					}
				until true end end
		if not prof then
			data[1] = {cells = {"The profiler is off in this build"}, colors = {C.inkMuted}}
		end
		render({
		{title = "Function", width = 0},
		{title = "Calls", width = 64, align = "RIGHT", sort = "calls"},
		{title = "Total", width = 72, align = "RIGHT", sort = "total"},
		{title = "Self", width = 72, align = "RIGHT", sort = "self"},
		{title = "Max", width = 72, align = "RIGHT", sort = "max"},
		{title = "Avg", width = 64, align = "RIGHT", sort = "avg"},
		{title = "", width = 44}
		}, data)
	end
	function views.monitor(d)
		local seg = tool("monSeg", function()
			local s = segmented(toolbar, {
			{value = "events", label = "Events"},
			{value = "scripts", label = "Scripts"},
			{value = "timers", label = "Timers"}
			}, function(v)
				state.monitor = v
				console.refresh(true)
			end)
			s:SetPoint("LEFT")
			return s
		end)
		seg:Select(state.monitor)
		tool("monReset", function()
			local b = button(toolbar, "Reset", function()
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
			for _, r in ipairs(console.sorted(mon[state.monitor], "total")) do
				local s = r.s
				local __wl_t24 = s.max if __wl_t24 == nil then __wl_t24 = 0 end local slow = (__wl_t24) > 16.7
				data[#data + 1] = {
				cells = {r.name, count(s.count), ms(s.total), ms(s.max)},
				subs = {s.detail},
				colors = {C.ink, C.ink, C.ink, slow and C.amber or C.ink},
				tall = s.detail ~= nil
				}
			end
		else
			data[1] = {cells = {"The monitor is off in this build"}, colors = {C.inkMuted}}
		end
		local head = ({events = "Event", scripts = "Frame · script", timers = "Timer"})[state.monitor]
		render({
		{title = head, width = 0},
		{title = state.monitor == "events" and "Count" or "Runs", width = 72, align = "RIGHT"},
		{title = "Total", width = 84, align = "RIGHT"},
		{title = "Max", width = 84, align = "RIGHT"}
		}, data)
	end
	local LEVEL_RANK = {trace = 0, debug = 1, info = 2, warn = 3, error = 4}
	function views.log(d)
		local seg = tool("logSeg", function()
			local s = segmented(toolbar, {
			{value = "all", label = "All"},
			{value = "info", label = "Info"},
			{value = "warn", label = "Warn"},
			{value = "error", label = "Error"}
			}, function(v)
				state.level = v
				console.refresh(true)
			end)
			s:SetPoint("LEFT")
			return s
		end)
		seg:Select(state.level)
		showTools({"logSeg"})
		local logs = d.runtime.logs
		local data = {}
		if logs then
			local min = state.level == "all" and -1 or LEVEL_RANK[state.level]
			for _, e in ipairs(logs.history()) do repeat
					do local __wl_t25 = LEVEL_RANK[e.level] if __wl_t25 == nil then __wl_t25 = 0 end if (__wl_t25) < min then
							break
						end end
					local tone = e.level == "error" and C.emberText or (e.level == "warn" and C.amber or C.inkMuted)
					do
						local __wl_t26 = e.time if __wl_t26 == nil then __wl_t26 = 0 end local __wl_t28 = string.format("%.1f", __wl_t26) local __wl_t27 = e.location if __wl_t27 == nil then __wl_t27 = "" end data[#data + 1] = {cells = {__wl_t28, e.level, e.message, __wl_t27},
						colors = {C.inkMuted, tone, C.ink, C.inkMuted},
						fonts = {"numeric", "display", "body", "body"}
						} end
				until true end
		else
			data[1] = {cells = {"", "", "This addon does not use wowlua:log"}, colors = {C.ink, C.ink, C.inkMuted}}
		end
		render({
		{title = "Time", width = 64},
		{title = "Level", width = 56},
		{title = "Message", width = 0},
		{title = "Location", width = 180}
		}, data)
	end
	local openError
	function views.errors(d)
		tool("errClear", function()
			local b = button(toolbar, "Clear", function()
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
				local open = openError == e
				data[#data + 1] = {
				cells = {(open and "- " or "+ ") .. e.message, "×" .. e.count, e.time},
				colors = {C.emberText, C.ink, C.inkMuted},
				onClick = function()
					if open then openError = nil else openError = e end
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
				data[1] = {cells = {"No errors"}, colors = {C.inkMuted}}
			end
		else
			data[1] = {cells = {"Error capture is off in this build"}, colors = {C.inkMuted}}
		end
		render({{title = "Error", width = 0}, {title = "", width = 48, align = "RIGHT"}, {title = "", width = 70, align = "RIGHT"}}, data)
	end
	local function valueText(ok, v)
		if not ok then
			return tostring(v), C.emberText
		end
		if type(v) == "table" and not (issecretvalue and issecretvalue(v)) then
			local n = 0
			for _ in pairs(v) do
				n = n + 1
			end
			return "table: " .. n .. " fields", C.arcane
		end
		local shown = console.show(v)
		return shown, shown == "<secret>" and C.inkMuted or C.ink
	end
	function views.watch(d)
		local input = tool("watchInput", function()
			local f = field(toolbar, 520, "Lua expression: ns.db.profile.scale, UnitHealth(\"player\")", function(expression)
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
			local b = button(toolbar, "Watch", function()
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
			colors = {C.ink, color, C.inkMuted},
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
			data[1] = {cells = {"No watches. Add one above, with debug.watch, or /wl watch."}, colors = {C.inkMuted}, fonts = {"body"}}
		end
		render({{title = "Watch", width = 0}, {title = "Value", width = 280}, {title = "From", width = 120}}, data)
	end
	function views.inspector(d)
		local _ = d
		showTools({})
		local data = {}
		local ok, root = pcall(inspected.get)
		local function walk(value, path, label, depth, seen, getter)
			local open = state.expanded[path]
			local isTable = type(value) == "table" and not (issecretvalue and issecretvalue(value))
			local shown, color = valueText(true, value)
			if isTable and seen[value] then
				shown, color = "(cycle)", C.inkMuted
				isTable = false
			end
			data[#data + 1] = {
			cells = {(isTable and (open and "- " or "+ ") or "   ") .. label, shown, type(value)},
			colors = {label:sub(1, 1) == "<" and C.inkMuted or C.ink, color, C.inkMuted},
			fonts = {"label", "numeric", "body"},
			indent = depth * 18,
			onClick = function()
				if isTable then
					state.expanded[path] = not open
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
			if state.expanded[""] == nil then
				state.expanded[""] = true
			end
			walk(root, "", inspected.label, 0, {}, inspected.get)
		else
			data[1] = {cells = {inspected.label, tostring(root)}, colors = {C.ink, C.emberText}}
		end
		render({{title = "Key", width = 0}, {title = "Value", width = 300}, {title = "Type", width = 80}}, data)
	end
	local VIEW_KEYS = {"overview", "profiler", "monitor", "log", "errors", "watch", "inspector"}

	function console.refresh()
		local d = current()
		if not d then
			return
		end
		selected = d.name
		addonLabel:SetText(d.name)
		for i, b in ipairs(tabButtons) do
			local active = i == state.tab
			b.fs:SetTextColor(unpack(active and C.giltText or C.inkMuted))
			b.line:SetShown(active)
			b.gem:SetShown(active)
		end
		local info = d.info
		footText:SetText((info.version or "?") .. " · " .. (info.profile or "?") .. " · build " .. (info.build or "?")
		.. " · wowlua " .. (info.compiler or "?"))
		views[VIEW_KEYS[state.tab]](d)
	end

	function console.setTab(i)
		state.tab = i
		console.refresh()
	end

	-- Saved per addon (`WowluaDevDB_<Addon>`), written to every loaded one.
	local function savedName(name)
		return "WowluaDevDB_" .. name:gsub("[^%w_]", "_")
	end
	function console.saveState()
		local point, _, _, px, py = win:GetPoint(1)
		local saved = {
		point = point, x = px, y = py,
		width = win:GetWidth(), height = win:GetHeight(),
		tab = state.tab, addon = selected
		}
		for _, n in ipairs(reg.order) do
			_G[savedName(n)] = saved
		end
	end
	for _, n in ipairs(reg.order) do
		local saved = _G[savedName(n)]
		if type(saved) == "table" then
			state.tab = saved.tab or state.tab
			selected = (saved.addon and reg.addons[saved.addon]) and saved.addon or selected
			if saved.point then
				win:ClearAllPoints()
				do local __wl_t29 = saved.x if __wl_t29 == nil then __wl_t29 = 0 end local __wl_t30 = saved.y if __wl_t30 == nil then __wl_t30 = 0 end win:SetPoint(saved.point, UIParent, saved.point, __wl_t29, __wl_t30) end
			end
			if saved.width and saved.height then
				win:SetSize(saved.width, saved.height)
			end
			break
		end
	end

	-- Refreshed only while visible: watches and the inspector often.
	local elapsed = 0
	win:SetScript("OnUpdate", function(_, dt)
		do local __wl_t31 = dt if __wl_t31 == nil then __wl_t31 = 0 end elapsed = elapsed + (__wl_t31) end
		local every = (state.tab == 6 or state.tab == 7) and 0.25 or 1
		if elapsed >= every and not menu:IsShown() then
			elapsed = 0
			console.refresh()
		end
	end)
	win:SetScript("OnHide", function()
		menu:Hide()
		addonList:Hide()
		console.saveState()
	end)
	return win
end
-- wowlua runtime: dev console (spec §7.7). This addon's dev data, the
-- shared `WOWLUA_DEV` registry, and — from the newest runtime loaded — the
-- `/wl` command and the dev window.
local dev
do

	local __wl_t32 = wl.devInfo if __wl_t32 == nil then __wl_t32 = {} end dev = {name = addonName, info = __wl_t32,
	watches = {},
	schema = 1,
	-- The data the console reads, through one versioned interface.
	runtime = wl,
	-- A watch from code (`debug.watch`): `fn()` shown live under `label`.
	watch = function(label, fn)
		dev.watches[#dev.watches + 1] = {label = label, fn = fn, source = "code"}
	end
	} end
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

local __wl_t33 = wl.devInfo if __wl_t33 ~= nil then __wl_t33 = __wl_t33.runtime end local __wl_t34 = __wl_t33 if __wl_t34 == nil then __wl_t34 = 0 end local VERSION = __wl_t34

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

	local function say(text)
		local frame = DEFAULT_CHAT_FRAME
		local line = "|cff33ff99wowlua|r " .. text
		if frame then
			frame:AddMessage(line)
		else
			print(line)
		end
	end
	console.say = say

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
	local function ms(v)
		return string.format(v >= 10 and "%.1f" or "%.2f", v)
	end

	-- Rows of a statistics table, sorted by `key` (descending).
	function console.sorted(stats, key)
		local rows = {}
		for name, s in pairs(stats or {}) do
			rows[#rows + 1] = {name = name, s = s}
		end
		table.sort(rows, function(a, b)
			do local __wl_t35 = a.s[key] if __wl_t35 == nil then __wl_t35 = 0 end local __wl_t36 = b.s[key] if __wl_t36 == nil then __wl_t36 = 0 end return (__wl_t35) > (__wl_t36) end
		end)
		return rows
	end

	local function statTable(stats, sortKey, top, countHead)
		local rows = console.sorted(stats, sortKey)
		say(pad("name", 34) .. pad(countHead, 8, true) .. pad("total", 9, true) .. pad("self", 9, true) .. pad("max", 9, true) .. pad("avg", 9, true))
		for i = 1, math.min(top, #rows) do
			local r = rows[i]
			local s = r.s
			do local __wl_t50 = pad(r.name, 34) local __wl_t37 = s.calls if __wl_t37 == nil then __wl_t37 = s.count end local __wl_t38 = __wl_t37 if __wl_t38 == nil then __wl_t38 = 0 end local __wl_t49 = pad(__wl_t38, 8, true) local __wl_t39 = s.total if __wl_t39 == nil then __wl_t39 = 0 end local __wl_t48 = pad(ms(__wl_t39), 9, true)
				local __wl_t40 = s.self if __wl_t40 == nil then __wl_t40 = s.total end local __wl_t41 = __wl_t40 if __wl_t41 == nil then __wl_t41 = 0 end local __wl_t47 = pad(ms(__wl_t41), 9, true) local __wl_t42 = s.max if __wl_t42 == nil then __wl_t42 = 0 end local __wl_t46 = pad(ms(__wl_t42), 9, true)
				local __wl_t43 = s.total if __wl_t43 == nil then __wl_t43 = 0 end local __wl_t44 = s.calls if __wl_t44 == nil then __wl_t44 = s.count end local __wl_t45 = __wl_t44 if __wl_t45 == nil then __wl_t45 = 1 end say(__wl_t50 .. __wl_t49 .. __wl_t48 .. __wl_t47 .. __wl_t46 .. pad(ms((__wl_t43) / math.max(__wl_t45, 1)), 9, true)) end
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
			do local __wl_t51 = i.version if __wl_t51 == nil then __wl_t51 = "?" end local __wl_t52 = i.profile if __wl_t52 == nil then __wl_t52 = "?" end local __wl_t53 = i.build if __wl_t53 == nil then __wl_t53 = "?" end say(tostring(n) .. " " .. tostring(__wl_t51) .. " · " .. tostring(__wl_t52) .. " · build " .. tostring(__wl_t53)) end
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
		say(tostring(d.name) .. " profiler (ms), by " .. tostring(sortKey) .. ":")
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
		for _, kind in ipairs({"events", "scripts", "timers"}) do
			say(tostring(d.name) .. " " .. tostring(kind) .. ":")
			statTable(mon[kind], "total", 10, kind == "events" and "count" or "runs")
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
			say("|cffff9a80" .. tostring(e.message) .. "|r ×" .. tostring(e.count) .. " · " .. tostring(e.time))
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
			return say(tostring(d.name) .. ": log level " .. tostring(args[2]:lower()))
		end
		for _, e in ipairs(logs.history(20)) do
			say(tostring(string.format("%.1f", e.time)) .. " [" .. tostring(e.level) .. "] " .. tostring(e.message))
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
			return say("|cffff9a80" .. tostring(problem) .. "|r")
		end
		d.watches[#d.watches + 1] = {label = expression, fn = fn, source = "typed"}
		local ok, value = pcall(fn)
		say(d.name .. " watch " .. expression .. " = " .. (ok and console.show(value) or ("|cffff9a80" .. tostring(value) .. "|r")))
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
		say("/wl [list | prof | mon | errors | log | meta | inspect | watch | reload]")
	end

	if type(SlashCmdList) == "table" then
		_G.SLASH_WOWLUA1 = "/wl"
		SlashCmdList.WOWLUA = function(msg)
			reg.console.command(msg)
		end
	end
	-- `/wl`: the dev window, built when first opened.
	if type(CreateFrame) == "function" then
		function console.toggle()
			if not console.window then
				console.window = buildDevWindow(console, reg)
			end
			local w = console.window
			if w:IsShown() then
				w:Hide()
			else
				w:Show()
				console.refresh()
			end
		end
	end
	dev.console = console
end
do local __wl_t54 = reg.version if __wl_t54 == nil then __wl_t54 = -1 end if (__wl_t54) < VERSION then
		install()
	end end
