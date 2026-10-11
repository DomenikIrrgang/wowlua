-- wowlua runtime: dev console (spec §7.7). This addon's dev data; the
-- `/wl` command and the dev window are shared by all loaded dev builds.
local dev = { watches = {} }
wl.dev = dev
-- A watch from code (`debug.watch`): `fn()` shown live under `label`.
function dev.watch(label, fn)
	dev.watches[#dev.watches + 1] = { label = label, fn = fn, source = "code" }
end
