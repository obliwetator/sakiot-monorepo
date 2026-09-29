import { useRef, useState } from "react";
import { loadClipBuffer } from "./useClipBuffer";

/**
 * Decoded audio for every source the editor's timeline, clipboard, and undo
 * history reference, and which of them are still loading.
 *
 * The editor owns these buffers. Shared-cache eviction must not remove them:
 * playback requires every source synchronously, including sources restored by
 * undo after cache pressure.
 */
export function useSourceBuffers() {
	const buffersRef = useRef<Map<string, AudioBuffer>>(new Map());
	const [loadingClips, setLoadingClips] = useState<Map<string, boolean>>(
		new Map(),
	);

	const markLoading = (sourceIds: string[]) => {
		setLoadingClips((previous) => {
			const next = new Map(previous);
			for (const sourceId of sourceIds) next.set(sourceId, true);
			return next;
		});
	};

	const markLoaded = (sourceId: string) => {
		setLoadingClips((previous) => {
			const next = new Map(previous);
			next.delete(sourceId);
			return next;
		});
	};

	const registerBuffer = (sourceId: string, buffer: AudioBuffer) => {
		buffersRef.current.set(sourceId, buffer);
	};

	/**
	 * Makes `sourceId` playable, then calls `onReady` - synchronously when it is
	 * already decoded. Resolves false, without calling `onReady`, when it cannot
	 * be loaded.
	 */
	const withSource = (
		guildId: string,
		sourceId: string,
		onReady: () => void,
	): Promise<boolean> => {
		if (buffersRef.current.has(sourceId)) {
			onReady();
			return Promise.resolve(true);
		}
		markLoading([sourceId]);
		return loadClipBuffer(guildId, sourceId)
			.then((buffer) => {
				registerBuffer(sourceId, buffer);
				onReady();
				return true;
			})
			.catch(() => false)
			.finally(() => markLoaded(sourceId));
	};

	const preloadSources = async (guildId: string, sourceIds: string[]) => {
		const missing = sourceIds.filter(
			(sourceId) => !buffersRef.current.has(sourceId),
		);
		if (missing.length === 0) return;
		markLoading(missing);
		await Promise.all(
			missing.map((sourceId) =>
				loadClipBuffer(guildId, sourceId)
					.then((buffer) => registerBuffer(sourceId, buffer))
					.catch(() => {
						// Segments referencing an unreadable source stay silent;
						// the edit itself remains intact for re-export.
					})
					.finally(() => markLoaded(sourceId)),
			),
		);
	};

	const sourceDuration = (sourceId: string): number | null =>
		buffersRef.current.get(sourceId)?.duration ?? null;

	return {
		buffersRef,
		loadingClips,
		registerBuffer,
		withSource,
		preloadSources,
		sourceDuration,
	};
}
