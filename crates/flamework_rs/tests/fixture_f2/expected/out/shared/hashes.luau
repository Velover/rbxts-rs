-- Compiled with roblox-ts v3.0.0
-- @metadata macro
local function hashOf(name, scoped)
	return { name, scoped }
end
-- `Modding.Hash` is a fresh hash on every clean build (random in TS5, derived from a per-build seed here).
local hashes = hashOf("19110e05-ff4c-5bcb-973e-115a6d519334", "4ba0b645-9238-5de9-8763-4eea3b62c5eb")
return {
	hashOf = hashOf,
	hashes = hashes,
}
