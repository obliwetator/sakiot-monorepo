export type SessionPlaybackTab = "normal" | "silence";
export type VisiblePlaybackTab = SessionPlaybackTab | "mix";

/**
 * The playback tab on screen.
 *
 * The selection controller owns which session timeline (normal or
 * silence-free) is active; the channel mix is an overlay the user chooses on
 * top of it. Deriving the visible tab from both, instead of mirroring the
 * controller into a second piece of state, means the controller can switch
 * timelines (silence removal finishing, or becoming unavailable) without an
 * effect having to copy the change across, and a mix that loses its tracks
 * falls back to the active timeline on the same render.
 */
export function visiblePlaybackTab(
	mixChosen: boolean,
	mixAvailable: boolean,
	sessionTab: SessionPlaybackTab,
): VisiblePlaybackTab {
	return mixChosen && mixAvailable ? "mix" : sessionTab;
}
