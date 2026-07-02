import Box from "@mui/material/Box";
import Tooltip from "@mui/material/Tooltip";
import { useEffect, useMemo, useRef, useState } from "react";
import type { VoiceEvent } from "../../../app/apiSlice";

const LABELS: Record<string, string> = {
	SERVER_MUTE: "Muted (server)",
	SERVER_UNMUTE: "Unmuted (server)",
	SERVER_DEAFEN: "Deafened (server)",
	SERVER_UNDEAFEN: "Undeafened (server)",
	SELF_MUTE: "Muted",
	SELF_UNMUTE: "Unmuted",
	SELF_DEAFEN: "Deafened",
	SELF_UNDEAFEN: "Undeafened",
	SUPPRESS_ON: "Suppressed",
	SUPPRESS_OFF: "Unsuppressed",
	STREAM_START: "Started streaming",
	STREAM_STOP: "Stopped streaming",
	VIDEO_ON: "Camera on",
	VIDEO_OFF: "Camera off",
	CHANNEL_JOIN: "Joined channel",
	CHANNEL_LEAVE: "Left channel",
	CHANNEL_SWITCH: "Switched channel",
	RECORDING_PAUSE: "Recording paused",
	RECORDING_RESUME: "Recording resumed",
};

// Distinct color per event family. Tweakable; chosen to read against MUI
// primary blue track without clashing.
//
// Legend:
//   #ef4444 red    — mute toggles (SERVER_MUTE / SERVER_UNMUTE / SELF_MUTE / SELF_UNMUTE)
//   #a855f7 purple — deafen toggles (SERVER_DEAFEN / SERVER_UNDEAFEN / SELF_DEAFEN / SELF_UNDEAFEN)
//   #06b6d4 cyan   — channel transitions (CHANNEL_JOIN / CHANNEL_LEAVE / CHANNEL_SWITCH)
//   #eab308 yellow — suppress toggles (SUPPRESS_ON / SUPPRESS_OFF)
//   #22c55e green  — stream / video toggles (STREAM_START/STOP, VIDEO_ON/OFF)
//   #f97316 orange — recording pause/resume on bot recoverable disconnect (RECORDING_PAUSE / RECORDING_RESUME)
//   #9ca3af gray   — fallback for unrecognized event_type strings
function colorFor(t: string): string {
	if (t.endsWith("MUTE") || t.endsWith("UNMUTE")) return "#ef4444";
	if (t.endsWith("DEAFEN") || t.endsWith("UNDEAFEN")) return "#a855f7";
	if (t.startsWith("CHANNEL")) return "#06b6d4";
	if (t.startsWith("SUPPRESS")) return "#eab308";
	if (t.startsWith("STREAM") || t.startsWith("VIDEO")) return "#22c55e";
	if (t.startsWith("RECORDING")) return "#f97316";
	return "#9ca3af";
}

function formatOffset(ms: number): string {
	if (ms < 0) return "--:--";
	const total = Math.floor(ms / 1000);
	const m = Math.floor(total / 60);
	const s = total % 60;
	return `${m}:${s.toString().padStart(2, "0")}`;
}

const MARKER_PX = 10;
const COLLISION_GAP_PX = 2;
const COLLISION_THRESHOLD_PX = MARKER_PX + COLLISION_GAP_PX;
const LANE_OFFSET_PX = 11;

type Placed = {
	event: VoiceEvent;
	pct: number;
	lane: number;
};

function assignLanes(
	events: VoiceEvent[],
	durationSec: number,
	widthPx: number,
): Placed[] {
	const visible = events
		.filter((e) => {
			const sec = e.offset_ms / 1000;
			return sec >= 0 && sec <= durationSec;
		})
		.slice()
		.sort((a, b) => a.offset_ms - b.offset_ms);

	if (widthPx <= 0 || visible.length === 0) return [];

	const lanesLastPx: number[] = [];
	const out: Placed[] = [];

	for (const e of visible) {
		const sec = e.offset_ms / 1000;
		const px = (sec / durationSec) * widthPx;
		const pct = (sec / durationSec) * 100;

		let assigned = -1;
		for (let i = 0; i < lanesLastPx.length; i++) {
			if (px - lanesLastPx[i] >= COLLISION_THRESHOLD_PX) {
				assigned = i;
				break;
			}
		}
		if (assigned === -1) {
			assigned = lanesLastPx.length;
			lanesLastPx.push(px);
		} else {
			lanesLastPx[assigned] = px;
		}
		out.push({ event: e, pct, lane: assigned });
	}
	return out;
}

export function VoiceEventMarkers(props: {
	events: VoiceEvent[];
	durationSec: number;
	audioRef: HTMLAudioElement;
}) {
	const containerRef = useRef<HTMLDivElement | null>(null);
	const [widthPx, setWidthPx] = useState(0);

	useEffect(() => {
		const el = containerRef.current;
		if (!el) return;
		setWidthPx(el.clientWidth);
		const ro = new ResizeObserver((entries) => {
			for (const entry of entries) {
				setWidthPx(entry.contentRect.width);
			}
		});
		ro.observe(el);
		return () => ro.disconnect();
	}, []);

	const placed = useMemo(
		() => assignLanes(props.events, props.durationSec, widthPx),
		[props.events, props.durationSec, widthPx],
	);

	if (!Number.isFinite(props.durationSec) || props.durationSec <= 0)
		return null;

	return (
		<Box
			ref={containerRef}
			sx={{
				position: "absolute",
				top: 0,
				left: 0,
				right: 0,
				bottom: 0,
				pointerEvents: "none",
			}}
		>
			{placed.map((p, _) => {
				const e = p.event;
				const sec = e.offset_ms / 1000;
				const label = LABELS[e.event_type] ?? e.event_type;
				const yOffsetPx = -p.lane * LANE_OFFSET_PX;
				return (
					<Tooltip
						key={`${e.user_id}-${e.offset_ms}-${e.event_type}`}
						title={`${label} @ ${formatOffset(e.offset_ms)}`}
						arrow
						disableInteractive
					>
						<Box
							onClick={() => {
								props.audioRef.currentTime = sec;
							}}
							onKeyDown={(ev) => {
								if (ev.key === "Enter" || ev.key === " ") {
									props.audioRef.currentTime = sec;
								}
							}}
							sx={{
								position: "absolute",
								left: `${p.pct}%`,
								top: "50%",
								transform: `translate(-50%, calc(-50% + ${yOffsetPx}px))`,
								width: MARKER_PX,
								height: MARKER_PX,
								borderRadius: "50%",
								bgcolor: colorFor(e.event_type),
								border: "1.5px solid rgba(255,255,255,0.85)",
								boxShadow: "0 0 2px rgba(0,0,0,0.6)",
								pointerEvents: "auto",
								cursor: "pointer",
								zIndex: 2 + p.lane,
								"&:hover": {
									transform: `translate(-50%, calc(-50% + ${yOffsetPx}px)) scale(1.4)`,
									zIndex: 50,
								},
								transition: "transform 80ms ease-out",
							}}
						/>
					</Tooltip>
				);
			})}
		</Box>
	);
}
