import { useEffect, useState } from "react";
import { useWebSocketStream } from "../../app/useWebSocketStream";
import type { Metrics, RecordingMetrics, VoiceState } from "./types";

export function useMetricsStream(enabled: boolean) {
	const [metrics, setMetrics] = useState<Metrics | null>(null);
	const [localUptime, setLocalUptime] = useState<number>(0);
	const [refreshCounter, setRefreshCounter] = useState(0);

	useWebSocketStream({
		enabled,
		url: "wss://dev.patrykstyla.com/api/dashboard/stream?name=global",
		subscribe: { action: "subscribe", topic: "global" },
		unsubscribe: { action: "unsubscribe", topic: "global" },
		onMessage: (data) => {
			if (data.event_type === "METRICS_UPDATE" && data.payload) {
				const m: Metrics = JSON.parse(data.payload);
				setMetrics(m);
				setLocalUptime(m.uptime_seconds);
			}
		},
		onAuthRefreshed: () => setRefreshCounter((p) => p + 1),
	});

	useEffect(() => {
		if (localUptime <= 0) return;
		const id = setInterval(() => setLocalUptime((p) => p + 1), 1000);
		return () => clearInterval(id);
	}, [localUptime]);

	return {
		metrics,
		localUptime,
		bumpRefresh: () => setRefreshCounter((p) => p + 1),
		refreshCounter,
	};
}

export function useGuildVoiceStream(
	enabled: boolean,
	guildId: string | null,
	_refreshCounter: number,
) {
	const [voiceUsers, setVoiceUsers] = useState<VoiceState[]>([]);
	const [userStartTimes, setUserStartTimes] = useState<Record<string, number>>(
		{},
	);
	const [guildRecordingMetrics, setGuildRecordingMetrics] =
		useState<RecordingMetrics | null>(null);
	const [, setLocalBump] = useState(0);

	useEffect(() => {
		if (!enabled || !guildId) {
			setVoiceUsers([]);
			setUserStartTimes({});
			setGuildRecordingMetrics(null);
		}
	}, [enabled, guildId]);

	useWebSocketStream({
		enabled: enabled && !!guildId,
		url: guildId
			? `wss://dev.patrykstyla.com/api/dashboard/stream?name=guild_voice_${guildId}`
			: null,
		subscribe: {
			action: "subscribe",
			topic: `guild_voice:${guildId}`,
		},
		unsubscribe: {
			action: "unsubscribe",
			topic: `guild_voice:${guildId}`,
		},
		onMessage: (data) => {
			if (data.event_type === "GUILD_VOICE_UPDATE" && data.payload) {
				const p = JSON.parse(data.payload);
				setVoiceUsers(p.voice_states);
				setUserStartTimes(p.user_start_times);
				if (p.recording_metrics) setGuildRecordingMetrics(p.recording_metrics);
			}
		},
		onAuthRefreshed: () => setLocalBump((p) => p + 1),
		deps: [guildId],
	});

	return { voiceUsers, userStartTimes, guildRecordingMetrics };
}

export function useNowTick() {
	const [currentTime, setCurrentTime] = useState<number>(
		Math.floor(Date.now() / 1000),
	);
	useEffect(() => {
		const id = setInterval(
			() => setCurrentTime(Math.floor(Date.now() / 1000)),
			1000,
		);
		return () => clearInterval(id);
	}, []);
	return currentTime;
}
