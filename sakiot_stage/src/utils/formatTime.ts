export function formatDuration(value: number) {
	if (!Number.isFinite(value) || Number.isNaN(value)) return "00:00:00";
	return new Date(Math.floor(value) * 1000).toISOString().slice(11, 19);
}

export function formatUptime(seconds: number) {
	const d = Math.floor(seconds / (3600 * 24));
	const h = Math.floor((seconds % (3600 * 24)) / 3600);
	const m = Math.floor((seconds % 3600) / 60);
	const s = Math.floor(seconds % 60);
	const dDisplay = d > 0 ? d + (d === 1 ? " day, " : " days, ") : "";
	const hDisplay = h > 0 ? h + (h === 1 ? " hour, " : " hours, ") : "";
	const mDisplay = m > 0 ? m + (m === 1 ? " minute, " : " minutes, ") : "";
	const sDisplay = s > 0 ? s + (s === 1 ? " second" : " seconds") : "";
	return dDisplay + hDisplay + mDisplay + sDisplay || "0 seconds";
}

export function formatTimeSince(
	timestampMs: number | undefined,
	currentUnixSecs: number,
): string {
	if (!timestampMs) return "Never";
	const seconds = Math.max(0, currentUnixSecs - Math.floor(timestampMs / 1000));
	return `${formatUptime(seconds)} ago`;
}

export function formatBytes(bytes: number): string {
	if (bytes >= 1024 * 1024 * 1024)
		return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
	return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
