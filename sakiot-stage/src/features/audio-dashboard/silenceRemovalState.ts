export interface SilenceRemovalStatus {
	status: "idle" | "processing" | "ready" | "failed";
	progress: number;
}

export function parseSilenceRemovalStatus(
	value: unknown,
): SilenceRemovalStatus {
	if (!value || typeof value !== "object") {
		return { status: "failed", progress: 0 };
	}
	const candidate = value as { status?: unknown; progress?: unknown };
	const status =
		candidate.status === "processing" ||
		candidate.status === "ready" ||
		candidate.status === "failed"
			? candidate.status
			: "idle";
	const progress =
		typeof candidate.progress === "number" &&
		Number.isFinite(candidate.progress)
			? Math.round(Math.min(100, Math.max(0, candidate.progress)))
			: 0;
	return { status, progress };
}
