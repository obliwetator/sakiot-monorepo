export type TimelineLaneId =
	| "recording"
	| "mute"
	| "deafen"
	| "suppress"
	| "channel"
	| "media"
	| "connection"
	| "other";

export interface AudioTimelineEvent {
	offset_ms: number;
	event_type: string;
	source?: string;
	user_id?: string;
	channel_id?: string | null;
	previous_channel_id?: string | null;
	details?: unknown;
}

export interface TimelinePoint {
	kind: "point";
	id: string;
	laneId: TimelineLaneId;
	track: number;
	offsetMs: number;
	label: string;
	color: string;
	event: AudioTimelineEvent;
}

export interface TimelineInterval {
	kind: "interval";
	id: string;
	laneId: TimelineLaneId;
	track: number;
	startMs: number;
	endMs: number;
	label: string;
	color: string;
	startEvent?: AudioTimelineEvent;
	endEvent?: AudioTimelineEvent;
	startsAtBoundary: boolean;
	endsAtBoundary: boolean;
}

export interface TimelineLane {
	id: TimelineLaneId;
	label: string;
	trackCount: number;
	points: TimelinePoint[];
	intervals: TimelineInterval[];
}

export interface EventTimelineModel {
	totalEvents: number;
	lanes: TimelineLane[];
}

export interface TimelinePointCluster {
	id: string;
	laneId: TimelineLaneId;
	track: number;
	offsetMs: number;
	startMs: number;
	endMs: number;
	points: TimelinePoint[];
}

type StateTransition = {
	phase: "start" | "end";
	stateKey: string;
	laneId: TimelineLaneId;
	track: number;
	label: string;
	color: string;
};

type IndexedEvent = {
	event: AudioTimelineEvent;
	index: number;
	type: string;
};

const LANE_DEFINITIONS: ReadonlyArray<{
	id: TimelineLaneId;
	label: string;
}> = [
	{ id: "recording", label: "Recording" },
	{ id: "mute", label: "Mute" },
	{ id: "deafen", label: "Deafen" },
	{ id: "suppress", label: "Suppress" },
	{ id: "channel", label: "Channel" },
	{ id: "media", label: "Media" },
	{ id: "connection", label: "Connection" },
	{ id: "other", label: "Other" },
];

const LABELS: Record<string, string> = {
	server_mute: "Server muted",
	server_unmute: "Server unmuted",
	server_deafen: "Server deafened",
	server_undeafen: "Server undeafened",
	self_mute: "Muted",
	self_unmute: "Unmuted",
	self_deafen: "Deafened",
	self_undeafen: "Undeafened",
	suppress_on: "Suppressed",
	suppress_off: "Unsuppressed",
	stream_start: "Stream started",
	stream_stop: "Stream stopped",
	video_on: "Camera on",
	video_off: "Camera off",
	channel_join: "Joined channel",
	channel_leave: "Left channel",
	channel_switch: "Switched channel",
	recording_pause: "Recording paused",
	recording_resume: "Recording resumed",
	user_recording_pause: "User recording paused",
	user_recording_resume: "User recording resumed",
	session_start: "Session started",
	pause: "Session paused",
	network_pause: "Network pause",
	disconnect: "Disconnected",
	afk: "Moved to AFK",
	resume: "Session resumed",
	timeout: "Session timed out",
	cap_expiry: "Session duration limit reached",
	writer_open: "Writer opened",
	writer_close: "Writer closed",
	writer_error: "Writer error",
	zombie_reaped: "Stale recording closed",
};

const STATE_TRANSITIONS: Record<string, StateTransition> = {
	server_mute: {
		phase: "start",
		stateKey: "server-mute",
		laneId: "mute",
		track: 0,
		label: "Server muted",
		color: "#ef4444",
	},
	server_unmute: {
		phase: "end",
		stateKey: "server-mute",
		laneId: "mute",
		track: 0,
		label: "Server muted",
		color: "#ef4444",
	},
	self_mute: {
		phase: "start",
		stateKey: "self-mute",
		laneId: "mute",
		track: 1,
		label: "Muted",
		color: "#fb7185",
	},
	self_unmute: {
		phase: "end",
		stateKey: "self-mute",
		laneId: "mute",
		track: 1,
		label: "Muted",
		color: "#fb7185",
	},
	server_deafen: {
		phase: "start",
		stateKey: "server-deafen",
		laneId: "deafen",
		track: 0,
		label: "Server deafened",
		color: "#a855f7",
	},
	server_undeafen: {
		phase: "end",
		stateKey: "server-deafen",
		laneId: "deafen",
		track: 0,
		label: "Server deafened",
		color: "#a855f7",
	},
	self_deafen: {
		phase: "start",
		stateKey: "self-deafen",
		laneId: "deafen",
		track: 1,
		label: "Deafened",
		color: "#c084fc",
	},
	self_undeafen: {
		phase: "end",
		stateKey: "self-deafen",
		laneId: "deafen",
		track: 1,
		label: "Deafened",
		color: "#c084fc",
	},
	suppress_on: {
		phase: "start",
		stateKey: "suppress",
		laneId: "suppress",
		track: 0,
		label: "Suppressed",
		color: "#eab308",
	},
	suppress_off: {
		phase: "end",
		stateKey: "suppress",
		laneId: "suppress",
		track: 0,
		label: "Suppressed",
		color: "#eab308",
	},
	stream_start: {
		phase: "start",
		stateKey: "stream",
		laneId: "media",
		track: 0,
		label: "Streaming",
		color: "#22c55e",
	},
	stream_stop: {
		phase: "end",
		stateKey: "stream",
		laneId: "media",
		track: 0,
		label: "Streaming",
		color: "#22c55e",
	},
	video_on: {
		phase: "start",
		stateKey: "video",
		laneId: "media",
		track: 1,
		label: "Camera on",
		color: "#14b8a6",
	},
	video_off: {
		phase: "end",
		stateKey: "video",
		laneId: "media",
		track: 1,
		label: "Camera on",
		color: "#14b8a6",
	},
	recording_pause: {
		phase: "start",
		stateKey: "recording-pause",
		laneId: "recording",
		track: 1,
		label: "Recording paused",
		color: "#f97316",
	},
	recording_resume: {
		phase: "end",
		stateKey: "recording-pause",
		laneId: "recording",
		track: 1,
		label: "Recording paused",
		color: "#f97316",
	},
	user_recording_pause: {
		phase: "start",
		stateKey: "user-recording-pause",
		laneId: "recording",
		track: 2,
		label: "User recording paused",
		color: "#fb923c",
	},
	user_recording_resume: {
		phase: "end",
		stateKey: "user-recording-pause",
		laneId: "recording",
		track: 2,
		label: "User recording paused",
		color: "#fb923c",
	},
	pause: {
		phase: "start",
		stateKey: "session-pause",
		laneId: "recording",
		track: 0,
		label: "Session paused",
		color: "#f97316",
	},
	network_pause: {
		phase: "start",
		stateKey: "session-pause",
		laneId: "recording",
		track: 0,
		label: "Network pause",
		color: "#f97316",
	},
	disconnect: {
		phase: "start",
		stateKey: "session-pause",
		laneId: "recording",
		track: 0,
		label: "Disconnected",
		color: "#f97316",
	},
	afk: {
		phase: "start",
		stateKey: "session-pause",
		laneId: "recording",
		track: 0,
		label: "Moved to AFK",
		color: "#f97316",
	},
	resume: {
		phase: "end",
		stateKey: "session-pause",
		laneId: "recording",
		track: 0,
		label: "Session paused",
		color: "#f97316",
	},
};

export function canonicalEventType(eventType: string): string {
	return eventType
		.trim()
		.toLowerCase()
		.replace(/[\s-]+/g, "_");
}

export function formatTimelineOffset(offsetMs: number): string {
	const totalSeconds = Math.max(0, Math.floor(offsetMs / 1_000));
	const hours = Math.floor(totalSeconds / 3_600);
	const minutes = Math.floor((totalSeconds % 3_600) / 60);
	const seconds = totalSeconds % 60;
	return [hours, minutes, seconds]
		.map((value) => value.toString().padStart(2, "0"))
		.join(":");
}

function humanizeEventType(eventType: string): string {
	const canonical = canonicalEventType(eventType);
	return (
		LABELS[canonical] ??
		canonical
			.split(/[_:]+/)
			.filter(Boolean)
			.map((word) => word[0]?.toUpperCase() + word.slice(1))
			.join(" ")
	);
}

function stateSubject(event: AudioTimelineEvent): string {
	return event.user_id ? `:${event.user_id}` : "";
}

function pointDescriptor(
	indexed: IndexedEvent,
	transition?: StateTransition,
): Omit<TimelinePoint, "kind" | "id" | "offsetMs" | "event"> {
	if (transition) {
		return {
			laneId: transition.laneId,
			track: transition.track,
			label: humanizeEventType(indexed.event.event_type),
			color: transition.color,
		};
	}

	const { event, type } = indexed;
	if (event.source === "voice_connection") {
		const failed = /(fail|error|timeout)/.test(type);
		return {
			laneId: "connection",
			track: 0,
			label: humanizeEventType(event.event_type),
			color: failed ? "#ef4444" : "#0ea5e9",
		};
	}
	if (type.startsWith("channel_")) {
		return {
			laneId: "channel",
			track: 0,
			label: humanizeEventType(event.event_type),
			color: "#06b6d4",
		};
	}
	if (
		type.startsWith("recording_") ||
		type.startsWith("user_recording_") ||
		type.startsWith("writer_") ||
		type === "session_start" ||
		type === "timeout" ||
		type === "cap_expiry" ||
		type === "zombie_reaped"
	) {
		const failed = /(error|timeout|expiry|zombie)/.test(type);
		return {
			laneId: "recording",
			track: 0,
			label: humanizeEventType(event.event_type),
			color: failed ? "#ef4444" : "#22c55e",
		};
	}
	return {
		laneId: "other",
		track: 0,
		label: humanizeEventType(event.event_type),
		color: "#94a3b8",
	};
}

function createPoint(
	indexed: IndexedEvent,
	transition?: StateTransition,
): TimelinePoint {
	return {
		kind: "point",
		id: `${indexed.index}:${indexed.event.offset_ms}:${indexed.type}`,
		offsetMs: indexed.event.offset_ms,
		event: indexed.event,
		...pointDescriptor(indexed, transition),
	};
}

function createInterval(args: {
	start?: IndexedEvent;
	end?: IndexedEvent;
	transition: StateTransition;
	durationMs: number;
}): TimelineInterval | null {
	const startMs = args.start?.event.offset_ms ?? 0;
	const endMs = args.end?.event.offset_ms ?? args.durationMs;
	if (endMs <= startMs) return null;
	const startId = args.start?.index ?? "boundary";
	const endId = args.end?.index ?? "boundary";
	return {
		kind: "interval",
		id: `${args.transition.stateKey}:${startId}:${endId}`,
		laneId: args.transition.laneId,
		track: args.transition.track,
		startMs,
		endMs,
		label: args.start
			? (STATE_TRANSITIONS[args.start.type]?.label ?? args.transition.label)
			: args.transition.label,
		color: args.start
			? (STATE_TRANSITIONS[args.start.type]?.color ?? args.transition.color)
			: args.transition.color,
		startEvent: args.start?.event,
		endEvent: args.end?.event,
		startsAtBoundary: !args.start,
		endsAtBoundary: !args.end,
	};
}

export function buildEventTimelineModel(
	events: readonly AudioTimelineEvent[],
	durationMs: number,
): EventTimelineModel {
	if (!Number.isFinite(durationMs) || durationMs <= 0) {
		return { totalEvents: 0, lanes: [] };
	}

	const visible: IndexedEvent[] = events
		.map((event, index) => ({
			event,
			index,
			type: canonicalEventType(event.event_type),
		}))
		.filter(
			({ event }) =>
				Number.isFinite(event.offset_ms) &&
				event.offset_ms >= 0 &&
				event.offset_ms <= durationMs,
		)
		.sort(
			(left, right) =>
				left.event.offset_ms - right.event.offset_ms ||
				left.index - right.index,
		);
	const points: TimelinePoint[] = [];
	const intervals: TimelineInterval[] = [];
	const activeStates = new Map<
		string,
		{ start: IndexedEvent; transition: StateTransition }
	>();

	for (const indexed of visible) {
		const transition = STATE_TRANSITIONS[indexed.type];
		if (!transition) {
			points.push(createPoint(indexed));
			continue;
		}

		const stateKey = transition.stateKey + stateSubject(indexed.event);
		if (transition.phase === "start") {
			if (activeStates.has(stateKey)) {
				points.push(createPoint(indexed, transition));
			} else {
				activeStates.set(stateKey, { start: indexed, transition });
			}
			continue;
		}

		const active = activeStates.get(stateKey);
		const interval = createInterval({
			start: active?.start,
			end: indexed,
			transition: active?.transition ?? transition,
			durationMs,
		});
		if (interval) {
			intervals.push(interval);
		} else {
			points.push(createPoint(indexed, transition));
		}
		activeStates.delete(stateKey);
	}

	for (const { start, transition } of activeStates.values()) {
		const interval = createInterval({
			start,
			transition,
			durationMs,
		});
		if (interval) {
			intervals.push(interval);
		} else {
			points.push(createPoint(start, transition));
		}
	}

	const lanes = LANE_DEFINITIONS.map(({ id, label }) => {
		const lanePoints = points
			.filter((point) => point.laneId === id)
			.sort((left, right) => left.offsetMs - right.offsetMs);
		const laneIntervals = intervals
			.filter((interval) => interval.laneId === id)
			.sort((left, right) => left.startMs - right.startMs);
		const maxTrack = Math.max(
			0,
			...lanePoints.map((point) => point.track),
			...laneIntervals.map((interval) => interval.track),
		);
		return {
			id,
			label,
			trackCount: maxTrack + 1,
			points: lanePoints,
			intervals: laneIntervals,
		};
	}).filter((lane) => lane.points.length > 0 || lane.intervals.length > 0);

	return { totalEvents: visible.length, lanes };
}

export function clusterTimelinePoints(
	points: readonly TimelinePoint[],
	durationMs: number,
	widthPx: number,
	minDistancePx = 14,
): TimelinePointCluster[] {
	if (points.length === 0) return [];

	const groups = new Map<string, TimelinePoint[]>();
	for (const point of points) {
		const key = `${point.laneId}:${point.track}`;
		const group = groups.get(key);
		if (group) group.push(point);
		else groups.set(key, [point]);
	}

	const clusters: TimelinePointCluster[] = [];
	for (const group of groups.values()) {
		const sorted = group
			.slice()
			.sort((left, right) => left.offsetMs - right.offsetMs);
		let current: TimelinePoint[] = [];
		let centerMs = 0;

		const flush = () => {
			if (current.length === 0) return;
			const first = current[0];
			const last = current[current.length - 1];
			clusters.push({
				id: current.map((point) => point.id).join("|"),
				laneId: first.laneId,
				track: first.track,
				offsetMs: Math.round(
					current.reduce((sum, point) => sum + point.offsetMs, 0) /
						current.length,
				),
				startMs: first.offsetMs,
				endMs: last.offsetMs,
				points: current,
			});
			current = [];
			centerMs = 0;
		};

		for (const point of sorted) {
			if (
				current.length === 0 ||
				widthPx <= 0 ||
				durationMs <= 0 ||
				(Math.abs(point.offsetMs - centerMs) / durationMs) * widthPx <=
					minDistancePx
			) {
				current.push(point);
				centerMs =
					current.reduce((sum, item) => sum + item.offsetMs, 0) /
					current.length;
			} else {
				flush();
				current = [point];
				centerMs = point.offsetMs;
			}
		}
		flush();
	}

	return clusters.sort(
		(left, right) => left.offsetMs - right.offsetMs || left.track - right.track,
	);
}
