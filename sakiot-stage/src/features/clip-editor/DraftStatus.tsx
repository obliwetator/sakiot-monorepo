import { AlertCircle, Check, CirclePause, LoaderCircle } from "lucide-react";
import type { ReactNode } from "react";
import { Button, Notice } from "../../shared/ui";
import type { DraftStatus } from "./draftPersistence";
import type { DraftSaveFailure } from "./draftStorage";
import type { UseDraftPersistenceReturn } from "./useDraftPersistence";

/**
 * Compact save state beside the editor controls. Committed edits save in the
 * same frame, so "Saving…" only shows while a drag or slider preview waits
 * for its trailing write. Narrow toolbars have no room for the words, so
 * they show the icon and keep the words for screen readers; failures and
 * pauses are also explained in the notice below the toolbar.
 */
export function DraftStatusIndicator(props: { status: DraftStatus }) {
	return (
		<span
			role="status"
			className="flex flex-none items-center gap-1 text-xs whitespace-nowrap"
		>
			{indicatorContent(props.status)}
		</span>
	);
}

function indicatorContent(status: DraftStatus): ReactNode {
	switch (status.kind) {
		case "idle":
			return null;
		case "saving":
			return (
				<IndicatorText
					className="text-muted"
					icon={
						<LoaderCircle
							aria-hidden="true"
							className="size-3.5 animate-spin motion-reduce:animate-none"
						/>
					}
					label="Saving…"
				/>
			);
		case "saved":
			return (
				<IndicatorText
					className="text-muted"
					icon={<Check aria-hidden="true" className="size-3.5" />}
					label="Saved on this device"
				/>
			);
		case "failed":
			return (
				<IndicatorText
					className="text-red-200"
					icon={<AlertCircle aria-hidden="true" className="size-3.5" />}
					label="Couldn't save this draft"
				/>
			);
		case "conflict":
		case "damaged":
			return (
				<IndicatorText
					className="text-amber-100"
					icon={<CirclePause aria-hidden="true" className="size-3.5" />}
					label="Saving paused"
				/>
			);
	}
}

/** Narrow toolbars show only the icon and keep the label for screen readers. */
function IndicatorText(props: {
	className: string;
	icon: ReactNode;
	label: string;
}) {
	return (
		<span className={`flex items-center gap-1 ${props.className}`}>
			{props.icon}
			<span className="max-[599px]:sr-only">{props.label}</span>
		</span>
	);
}

const FAILURE_REASONS: Record<DraftSaveFailure, string> = {
	quota: "This browser's storage for the site is full.",
	unavailable: "This browser is blocking site storage.",
	failed: "The browser refused the write.",
};

/**
 * What needs the user's decision about the draft: a failed save, a conflict
 * with another tab, an unreadable stored draft, or a draft left by the
 * earlier editor. Renders nothing when there is none.
 */
export function DraftNotices(props: { draft: UseDraftPersistenceReturn }) {
	const { draft } = props;
	const { status } = draft;
	const notices: ReactNode[] = [];
	if (status.kind === "failed") {
		notices.push(
			<Notice key="failed" tone="error" announce="alert">
				<p>
					Couldn't save this draft. {FAILURE_REASONS[status.reason]} Your edit
					is still open here: retry, or download a copy.
				</p>
				<NoticeActions>
					<Button size="sm" variant="primary" onPress={draft.retry}>
						Retry
					</Button>
					<Button size="sm" variant="outline" onPress={draft.download}>
						Download draft
					</Button>
				</NoticeActions>
			</Notice>,
		);
	}
	if (status.kind === "conflict") {
		notices.push(
			<Notice key="conflict" tone="warning" announce="alert">
				<p>
					This draft was changed in another tab. Saving is paused here so
					neither version overwrites the other.
					{status.otherVersionReadable
						? " Choose which version to keep."
						: " The other tab's version can't be read."}
				</p>
				<NoticeActions>
					{status.otherVersionReadable && (
						<Button
							size="sm"
							variant="primary"
							onPress={draft.loadOtherVersion}
						>
							Load other tab's version
						</Button>
					)}
					<Button size="sm" variant="outline" onPress={draft.keepThisVersion}>
						Keep this tab's version
					</Button>
					<Button size="sm" variant="ghost" onPress={draft.download}>
						Download this version
					</Button>
				</NoticeActions>
			</Notice>,
		);
	}
	if (status.kind === "damaged") {
		notices.push(
			<Notice key="damaged" tone="error" announce="alert">
				<p>
					The draft saved on this device can't be read. Saving is paused so it
					isn't overwritten.
				</p>
				<NoticeActions>
					<Button size="sm" variant="outline" onPress={draft.downloadDamaged}>
						Download saved data
					</Button>
					<Button size="sm" variant="danger" onPress={draft.replaceDamaged}>
						Replace it with this edit
					</Button>
				</NoticeActions>
			</Notice>,
		);
	}
	if (draft.legacyDraftOffered) {
		notices.push(
			<Notice key="legacy" tone="info" announce="status">
				<p>
					This device has a draft from the earlier editor. Loading it replaces
					the current edit, and undo brings the current edit back.
				</p>
				<NoticeActions>
					<Button size="sm" variant="primary" onPress={draft.loadLegacyDraft}>
						Load earlier draft
					</Button>
					<Button size="sm" variant="ghost" onPress={draft.dismissLegacyDraft}>
						Dismiss
					</Button>
				</NoticeActions>
			</Notice>,
		);
	}
	if (notices.length === 0) return null;
	return (
		<div className="flex-none space-y-2 border-b border-ui-border px-2 py-2 min-[600px]:px-4">
			{notices}
		</div>
	);
}

function NoticeActions(props: { children: ReactNode }) {
	return <div className="mt-2 flex flex-wrap gap-2">{props.children}</div>;
}
