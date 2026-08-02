export function formatDuration(value: number) {
	if (!Number.isFinite(value) || Number.isNaN(value)) return "00:00:00";
	return new Date(Math.floor(value) * 1000).toISOString().slice(11, 19);
}

/** MM:SS for short recordings and HH:MM:SS once the session reaches an hour. */
export function formatSessionTimecode(
	valueSeconds: number,
	durationSeconds: number,
): string {
	const value = Number.isFinite(valueSeconds)
		? Math.max(0, Math.floor(valueSeconds))
		: 0;
	const hours = Math.floor(value / 3_600);
	const minutes = Math.floor((value % 3_600) / 60);
	const seconds = value % 60;
	const showHours =
		Number.isFinite(durationSeconds) && durationSeconds >= 3_600;
	return showHours
		? `${String(hours).padStart(2, "0")}:${String(minutes).padStart(
				2,
				"0",
			)}:${String(seconds).padStart(2, "0")}`
		: `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(
				seconds,
			).padStart(2, "0")}`;
}

/** Parses the duration-aware timecode accepted by the manual session seek. */
export function parseSessionTimecode(
	input: string,
	durationSeconds: number,
): number | null {
	if (!Number.isFinite(durationSeconds) || durationSeconds < 0) return null;
	const parts = input.trim().split(":");
	const showHours = durationSeconds >= 3_600;
	if (parts.length !== (showHours ? 3 : 2)) return null;
	if (parts.some((part) => !/^\d+$/.test(part))) return null;

	const values = parts.map(Number);
	const hours = showHours ? values[0] : 0;
	const minutes = showHours ? values[1] : values[0];
	const seconds = showHours ? values[2] : values[1];
	if (minutes >= 60 || seconds >= 60) return null;
	const totalSeconds = hours * 3_600 + minutes * 60 + seconds;
	return totalSeconds <= durationSeconds ? totalSeconds : null;
}

/** HH:MM:SS.t — for readouts where whole seconds are too coarse to act on. */
export function formatDurationPrecise(value: number) {
	if (!Number.isFinite(value) || Number.isNaN(value)) return "00:00:00.0";
	const clamped = Math.max(0, value);
	const tenths = Math.floor(clamped * 10) % 10;
	return `${formatDuration(clamped)}.${tenths}`;
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
