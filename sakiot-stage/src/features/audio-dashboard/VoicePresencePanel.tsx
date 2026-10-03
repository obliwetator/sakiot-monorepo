import {
	Bot as BotIcon,
	HeadphoneOff as HeadphoneOffIcon,
	MicOff as MicOffIcon,
	MonitorUp as MonitorUpIcon,
	Video as VideoIcon,
	Volume2 as Volume2Icon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useParams } from "react-router-dom";
import { useGetVoicePresenceQuery } from "../../app/apiSlice";
import { useAsRole } from "../../app/useAsRole";
import { useRealtimeLive } from "../../realtime/status";

type Member = NonNullable<
	ReturnType<typeof useGetVoicePresenceQuery>["data"]
>["channels"][number]["members"][number];

function StateIcon(props: { label: string; children: ReactNode }) {
	return (
		<span role="img" aria-label={props.label} title={props.label}>
			{props.children}
		</span>
	);
}

function MemberState({ member }: { member: Member }) {
	const deafened = member.self_deaf || member.server_deaf;
	const muted = member.self_mute || member.server_mute;
	return (
		<span className="flex items-center gap-1 text-muted shrink-0">
			{member.is_bot && (
				<StateIcon label="Bot">
					<BotIcon size={14} />
				</StateIcon>
			)}
			{member.streaming && (
				<StateIcon label="Streaming">
					<MonitorUpIcon size={14} />
				</StateIcon>
			)}
			{member.video && (
				<StateIcon label="Camera on">
					<VideoIcon size={14} />
				</StateIcon>
			)}
			{deafened ? (
				<StateIcon label="Deafened">
					<HeadphoneOffIcon size={14} />
				</StateIcon>
			) : (
				muted && (
					<StateIcon label="Muted">
						<MicOffIcon size={14} />
					</StateIcon>
				)
			)}
		</span>
	);
}

/**
 * Who is in the voice and stage channels the viewer can view, as Discord's
 * channel list shows it. Realtime refreshes it on every change; without
 * realtime it polls every 10 s. While live it still rechecks every minute,
 * since a bot that crashes sends no event and presence then turns unknown.
 */
export function VoicePresencePanel() {
	const params = useParams();
	const { asRoleArg } = useAsRole();
	const live = useRealtimeLive();
	const { currentData: presence } = useGetVoicePresenceQuery(
		{ guild_id: params.guild_id ?? "", ...asRoleArg },
		{ skip: !params.guild_id, pollingInterval: live ? 60_000 : 10_000 },
	);
	if (!presence) return null;

	return (
		<section aria-labelledby="voice-presence-heading" className="mb-3 px-1">
			<h2
				id="voice-presence-heading"
				className="text-xs font-semibold uppercase tracking-wide text-muted mb-1"
			>
				In voice
			</h2>
			{!presence.available ? (
				<p className="text-sm text-muted">Voice activity is unavailable.</p>
			) : presence.channels.length === 0 ? (
				<p className="text-sm text-muted">Nobody is in voice.</p>
			) : (
				<ul className="space-y-2">
					{presence.channels.map((channel) => (
						<li key={channel.channel_id}>
							<span className="flex items-center gap-1.5 text-sm font-medium">
								<Volume2Icon size={14} aria-hidden="true" />
								{channel.name}
							</span>
							<ul aria-label={`In ${channel.name}`} className="ml-5">
								{channel.members.map((member) => (
									<li
										key={member.user_id}
										className="flex items-center justify-between gap-2 text-sm"
									>
										<span className="truncate">
											{member.name ?? member.user_id}
										</span>
										<MemberState member={member} />
									</li>
								))}
							</ul>
						</li>
					))}
				</ul>
			)}
		</section>
	);
}
