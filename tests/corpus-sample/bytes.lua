-- CRLF line endings and non-UTF-8 bytes: cafÃ© (UTF-8), café (Latin-1)
local utf8 = "cafÃ©"
local latin = "café à la carte"
local t = { -- § comment
	a = 1,
}
return utf8, latin, t
