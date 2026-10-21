// Preload (node -r): prints peak resident memory (Windows: PeakWorkingSetSize) and wall time at exit.
const start = process.hrtime.bigint();
process.on("exit", () => {
	const ms = Number(process.hrtime.bigint() - start) / 1e6;
	process.stderr.write(`[maxrss] peak ${(process.resourceUsage().maxRSS / 1024).toFixed(1)} MiB, wall ${ms.toFixed(0)} ms\n`);
});
