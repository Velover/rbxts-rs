-- Compiled with roblox-ts v3.0.0
local TS = require(game:GetService("ReplicatedStorage"):WaitForChild("rbxts_include"):WaitForChild("RuntimeLib"))
local _markers = TS.import(script, game:GetService("ReplicatedStorage"), "TS", "markers")
local describe = _markers.describe
local fieldInfo = _markers.fieldInfo
local globOf = _markers.globOf
local memberCount = _markers.memberCount
local pathOf = _markers.pathOf
local transformerVersion = _markers.transformerVersion
-- Paths and globs.
local commands = pathOf("src/server/commands", { "ServerScriptService", "TS", "commands" })
local commandGlob = globOf("src/server/commands/*.ts", "src/server/commands/*.ts")
local relativeGlob = globOf("../server/commands/*.ts", "src/server/commands/*.ts")
-- Flamework.env: set in the fixture's .env, a fallback, and nothing.
local channel = "beta"
local region = "eu"
local missing = nil
-- Macro types of the fixture plugin.
local fieldInfo_1 = { {
	name = "fxcoins",
	kind = "number",
	optional = false,
	readonly = false,
}, {
	name = "fxuserId",
	kind = "number",
	optional = false,
	readonly = true,
}, {
	name = "fxnickname",
	kind = "union(string|undefined)",
	optional = true,
	readonly = false,
}, {
	name = "fxtags",
	kind = "array<string>",
	optional = false,
	readonly = false,
}, {
	name = "fxpair",
	kind = "tuple[string,number]",
	optional = false,
	readonly = false,
} }
local fields = fieldInfo(fieldInfo_1)
local fieldsAgain = fieldInfo(fieldInfo_1)
local literal = describe("string:hello")
local numberLiteral = describe("number:42")
local booleanLiteral = describe("boolean:true")
local kinds = describe("array<boolean>")
local members = memberCount(3)
local version = transformerVersion("2.0.0-alpha.8")
local function later()
	-- A second use in another statement reuses the hoisted value.
	return fieldInfo(fieldInfo_1)
end
return {
	later = later,
	commands = commands,
	commandGlob = commandGlob,
	relativeGlob = relativeGlob,
	channel = channel,
	region = region,
	missing = missing,
	fields = fields,
	fieldsAgain = fieldsAgain,
	literal = literal,
	numberLiteral = numberLiteral,
	booleanLiteral = booleanLiteral,
	kinds = kinds,
	members = members,
	version = version,
}
