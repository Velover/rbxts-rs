return {
	calls = {
		{ call = "pushRebinds", expect = { 4, 1 } },
		{ call = "subRebinds", expect = "bc" },
		{ call = "mathMaxRebinds", expect = 2 },
		{ call = "bitOrRebinds", expect = 3 },
		{ call = "tonumberRebinds", expect = 2 },
		{ call = "mathElementRebinds", expect = 2 },
		{ call = "mathMaxFour", expect = 2 },
		{ call = "mathMaxPlain", expect = 3 },
		{ call = "pushPlain", expect = 2 },
		{ call = "negativeZeroLoop", expect = -math.huge },
		{ call = "negativeZeroConstantLoop", expect = -math.huge },
	},
}
