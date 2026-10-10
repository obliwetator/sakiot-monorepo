/**
 * An audio element that AudioInterface's effects create and own. The handle
 * is React state, which the React Compiler treats as immutable, so the range
 * slider's controls change the element through these commands instead of
 * assigning its properties.
 */
export interface AudioHandle {
	element: HTMLAudioElement;
	seek: (seconds: number) => void;
	setVolume: (volume: number) => void;
	setMuted: (muted: boolean) => void;
	setPlaybackRate: (rate: number) => void;
}

export function audioHandle(element: HTMLAudioElement): AudioHandle {
	return {
		element,
		seek: (seconds) => {
			element.currentTime = seconds;
		},
		setVolume: (volume) => {
			element.volume = volume;
		},
		setMuted: (muted) => {
			element.muted = muted;
		},
		setPlaybackRate: (rate) => {
			element.playbackRate = rate;
		},
	};
}
