/**
 * The observable half of a playback engine, shaped for React's
 * `useSyncExternalStore`: `getSnapshot` returns the same object until a
 * published value actually changes, and `subscribe`/`getSnapshot` are bound so
 * they can be passed straight to the hook.
 */
export class PlaybackStore<Snapshot extends object> {
	private readonly listeners = new Set<() => void>();

	constructor(protected snapshot: Snapshot) {}

	readonly subscribe = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	readonly getSnapshot = (): Snapshot => this.snapshot;

	protected publish(patch: Partial<Snapshot>): void {
		let changed = false;
		for (const key in patch) {
			if (!Object.is(patch[key], this.snapshot[key])) {
				changed = true;
				break;
			}
		}
		if (!changed) return;
		this.snapshot = { ...this.snapshot, ...patch };
		for (const listener of this.listeners) listener();
	}
}
