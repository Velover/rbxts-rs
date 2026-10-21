-- Compiled with roblox-ts v3.0.0
-- No Flamework at all: compiled exactly as without the plugin.
local function add(a, b)
	return a + b
end
local Plain
do
	Plain = setmetatable({}, {
		__tostring = function()
			return "Plain"
		end,
	})
	Plain.__index = Plain
	function Plain.new(...)
		local self = setmetatable({}, Plain)
		return self:constructor(...) or self
	end
	function Plain:constructor()
		self.value = add(1, 2)
	end
end
return {
	add = add,
	Plain = Plain,
}
