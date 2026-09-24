import { Button, Notice, Tooltip, TooltipTrigger } from "../../shared/ui";
import { ChannelMixPlayer, ChannelMixProgress } from "./ChannelMixPlayer";
import type { useChannelMixPreferences } from "./channelMixPreferences";
import type { PlaybackShortcutTarget } from "./playbackShortcuts";
import type { SessionChannelMix } from "./useSessionChannelMix";

export function ChannelMixPanel(props: {
	sessionId: string;
	channelMix: SessionChannelMix;
	preferences: ReturnType<typeof useChannelMixPreferences>;
	volume: number;
	playbackRate: number;
	onVolumeChange: (volume: number) => void;
	onPlaybackRateChange: (rate: number) => void;
	onBeforePlay: () => void;
	onRegisterStop: (stop: () => void) => void;
	onPlaybackUse: (target: PlaybackShortcutTarget) => void;
	onPlaybackClear: (targetId: string) => void;
}) {
	const { mix, canGenerate, renderDirty } = props.channelMix;
	const generate = () => void props.channelMix.generate();
	return (
		<div className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-4 mt-3">
			<div className="flex flex-col justify-between items-start min-[600px]:items-center min-[600px]:flex-row gap-2">
				<div>
					<h6 className="leading-6 font-semibold tracking-tight font-medium tracking-[0.001em] leading-[1.6] text-xl">
						Channel mix
					</h6>
					<p className="leading-6 text-muted text-sm">
						{props.preferences.options.scope === "all_recordings"
							? "All recordings while the bot was continuously connected to this channel are shown on one timeline."
							: "Only recordings overlapping this selected session are shown on one timeline (anchor-style)."}
					</p>
				</div>
				<TooltipTrigger delay={400}>
					<Button
						variant="primary"
						isDisabled={!canGenerate || props.channelMix.generating}
						onPress={generate}
					>
						{props.channelMix.generating
							? "Starting…"
							: renderDirty || mix?.status === "ready"
								? "Regenerate channel mix"
								: mix?.status === "failed"
									? "Retry mix"
									: "Generate channel mix"}
					</Button>
					<Tooltip>
						{mix?.reason?.message ??
							(mix?.can_generate === false
								? "Every recording in this mix must be finalized"
								: undefined)}
					</Tooltip>
				</TooltipTrigger>
			</div>

			{props.channelMix.statusError && !mix && (
				<Notice className="mt-3" tone={"error"} announce="alert">
					Channel mix status is unavailable.{" "}
					<Button
						variant="primary"
						size="sm"
						onPress={() => void props.channelMix.refetch()}
					>
						Retry status
					</Button>
				</Notice>
			)}
			{mix && (
				<>
					{mix.reason && mix.status !== "ready" && (
						<Notice
							className="mt-3"
							tone={mix.status === "failed" ? "error" : "info"}
							announce={mix.status === "failed" ? "alert" : "status"}
						>
							{mix.reason.message}
						</Notice>
					)}
					{props.channelMix.processing && (
						<ChannelMixProgress progress={mix.progress} />
					)}
					{mix.status === "idle" && (
						<p className="leading-6 text-muted text-sm mt-2">
							{mix.source_count} source recordings found. The mix is ready to
							generate.
						</p>
					)}
					{mix.tracks.length > 0 && (
						<ChannelMixPlayer
							sessionId={props.sessionId}
							mix={mix}
							settings={props.channelMix.draft.settings}
							onSettingsChange={props.channelMix.draft.setSettings}
							options={props.preferences.options}
							onOptionsChange={props.preferences.setOptions}
							dialogOpen={props.preferences.dialogOpen}
							onDialogOpenChange={props.preferences.setDialogOpen}
							volume={props.volume}
							playbackRate={props.playbackRate}
							onVolumeChange={props.onVolumeChange}
							onPlaybackRateChange={props.onPlaybackRateChange}
							onBeforePlay={props.onBeforePlay}
							onRegisterStop={props.onRegisterStop}
							onPlaybackUse={props.onPlaybackUse}
							onPlaybackClear={props.onPlaybackClear}
							onGenerate={canGenerate ? generate : undefined}
						/>
					)}
				</>
			)}
			{props.channelMix.actionError && (
				<Notice className="mt-2" tone={"error"} announce="alert">
					{props.channelMix.actionError}
				</Notice>
			)}
		</div>
	);
}
