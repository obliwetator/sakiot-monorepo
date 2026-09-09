/**
 * The seek position requested by a `?t=` deep link, or `null` when the
 * parameter is absent or not a finite number.
 *
 * `Number.parseFloat("abc")` is `NaN`, and assigning a non-finite value to
 * `HTMLMediaElement.currentTime` throws a TypeError. That throw happens inside
 * the `canplay` handler, so an invalid `?t=` would abort the handler before
 * `setReadyToPlay(true)` and leave the player stuck.
 */
export function deepLinkSeekSeconds(search: string): number | null {
	const raw = new URLSearchParams(search).get("t");
	if (raw === null || raw.trim() === "") return null;
	const seconds = Number.parseFloat(raw);
	return Number.isFinite(seconds) ? seconds : null;
}
