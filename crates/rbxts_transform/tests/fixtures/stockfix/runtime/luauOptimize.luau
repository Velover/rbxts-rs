return {
	calls = {
		{ call = "pushRebinds", expect = { 4, 1 } },
		{ call = "subRebinds", expect = "bc" },
		{ call = "pushPlain", expect = 2 },
		{ call = "negativeZeroLoop", expect = -math.huge },
		{ call = "negativeZeroConstantLoop", expect = -math.huge },
	},
}
