import { type ClipData, useEraseClipMutation } from "../../app/apiSlice";
import {
	EraseButton,
	erasureDay,
	UndoErasureButton,
} from "../../shared/Erasure";
import { Notice } from "../../shared/ui";

/** Whether `userId`'s voice is in the clip, which makes it theirs to erase. */
export function holdsVoiceOf(clip: ClipData, userId: string | null): boolean {
	return userId !== null && clip.speaker_ids.includes(userId);
}

/** Offers a speaker to erase a clip of their voice. */
export function EraseClipControl(props: {
	clip: ClipData;
	currentUserId: string | null;
}) {
	const [erase, state] = useEraseClipMutation();
	if (props.clip.erasure || !holdsVoiceOf(props.clip, props.currentUserId)) {
		return null;
	}
	return (
		<EraseButton
			label="Erase my voice"
			title="Erase this clip of your voice?"
			busy={state.isLoading}
			erase={() =>
				erase({
					guild_id: props.clip.guild_id,
					clip_id: props.clip.clip_id,
				}).unwrap()
			}
		>
			<p className="text-sm leading-6 text-slate-200">
				Everyone else stops seeing it now, with every clip made from it. You can
				still play it until it is deleted.
			</p>
		</EraseButton>
	);
}

/** The speaker's erasure of the clip, while it is under way. */
export function ClipErasureNotice(props: { clip: ClipData }) {
	const erasure = props.clip.erasure;
	if (!erasure) return null;
	if (erasure.state !== "grace") {
		return (
			<Notice className="mb-4">This clip is being deleted for good.</Notice>
		);
	}
	return (
		<div className="mb-4 space-y-2">
			<Notice tone="warning">
				Deleting on {erasureDay(erasure.purge_at_ms)}. Only you can see this
				clip until then.
			</Notice>
			<UndoErasureButton guildId={props.clip.guild_id} erasureId={erasure.id} />
		</div>
	);
}
