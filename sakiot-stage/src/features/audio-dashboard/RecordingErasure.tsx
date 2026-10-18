import { type Removal, useEraseSessionMutation } from "../../app/apiSlice";
import {
	EraseButton,
	erasureDay,
	UndoErasureButton,
} from "../../shared/Erasure";
import { Notice } from "../../shared/ui";

/** The speaker erases their own recording. */
export function EraseRecordingControl(props: {
	guildId: string;
	sessionId: string;
	/** Whether they can still play it meanwhile. */
	playable: boolean;
}) {
	const [erase, state] = useEraseSessionMutation();
	return (
		<EraseButton
			label="Erase permanently"
			title="Erase this recording of your voice?"
			busy={state.isLoading}
			erase={() =>
				erase({
					guild_id: props.guildId,
					recording_session_id: props.sessionId,
				}).unwrap()
			}
		>
			<p className="text-sm leading-6 text-slate-200">
				Everyone else stops seeing it now, with every clip made from it.
				{props.playable && " You can still play it until it is deleted."}
			</p>
		</EraseButton>
	);
}

/**
 * What the speaker sees of their own recording once nobody else does:
 * removed by the server, which they can only erase, or their own erasure,
 * which they can undo until it is deleted.
 */
export function RecordingRemovalNotice(props: {
	guildId: string;
	sessionId: string;
	removal: Removal;
}) {
	const erasure = props.removal.erasure;
	if (!erasure) {
		return (
			<div className="mb-4 space-y-2">
				<Notice tone="warning">
					Removed by the server. Only you can still see this recording, and it
					cannot be played. You can erase it for good.
				</Notice>
				<EraseRecordingControl
					guildId={props.guildId}
					sessionId={props.sessionId}
					playable={false}
				/>
			</div>
		);
	}
	if (erasure.state !== "grace") {
		return (
			<Notice className="mb-4">
				This recording is being deleted for good.
			</Notice>
		);
	}
	return (
		<div className="mb-4 space-y-2">
			<Notice tone="warning">
				Deleting on {erasureDay(erasure.purge_at_ms)}. Only you can see this
				recording until then
				{props.removal.playable
					? "."
					: ". It was removed by the server, so it cannot be played."}
			</Notice>
			<UndoErasureButton guildId={props.guildId} erasureId={erasure.id} />
		</div>
	);
}
