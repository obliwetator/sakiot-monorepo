import { Monitor, Smartphone, Tablet } from "lucide-react";
import { useState } from "react";
import { problemFromError } from "../app/apiError";
import { useEndSessionMutation, useGetSessionsQuery } from "../app/apiSlice";
import { BaseDialog } from "../shared/BaseDialog";
import { Badge, Button } from "../shared/ui";
import { deviceKind, deviceLabel, lastActiveLabel } from "./sessions";

const ICONS = { phone: Smartphone, tablet: Tablet, computer: Monitor };

const signedInDate = new Intl.DateTimeFormat(undefined, {
	day: "numeric",
	month: "short",
	year: "numeric",
});

/** Mounted while open, so "Active 3 hours ago" counts from opening it. */
export function SessionsDialog(props: { onClose: () => void }) {
	const {
		data: sessions,
		error: listError,
		isLoading,
	} = useGetSessionsQuery(undefined, { refetchOnMountOrArgChange: true });
	const [endSession, { isLoading: ending }] = useEndSessionMutation();
	const [endError, setEndError] = useState<string | null>(null);
	const [now] = useState(Date.now);

	const end = async (sessionId: string) => {
		setEndError(null);
		try {
			await endSession(sessionId).unwrap();
		} catch (error) {
			setEndError(
				`Could not log that device out. ${problemFromError(error).message}`,
			);
		}
	};

	return (
		<BaseDialog
			open
			onClose={props.onClose}
			title="Signed-in devices"
			closeLabel="Close"
			error={
				listError
					? `Could not load your devices. ${problemFromError(listError).message}`
					: (endError ?? undefined)
			}
		>
			<p className="text-muted text-sm">
				Logging a device out takes effect within 15 minutes.
			</p>
			{isLoading && <p role="status">Loading devices…</p>}
			{sessions && (
				<ul aria-label="Signed-in devices" className="space-y-2">
					{sessions.map((session) => {
						const Icon = ICONS[deviceKind(session.platform)];
						const label = deviceLabel(session);
						return (
							<li
								key={session.id}
								className="flex items-center gap-3 rounded-md border border-ui-border p-3"
							>
								<Icon aria-hidden="true" size={20} className="shrink-0" />
								<div className="min-w-0 flex-1">
									<p className="font-medium">{label}</p>
									<p className="text-muted text-sm">
										{lastActiveLabel(session.last_active_at, now)} · Signed in{" "}
										{signedInDate.format(new Date(session.signed_in_at))}
									</p>
								</div>
								{session.current ? (
									<Badge tone="success" size="sm">
										This device
									</Badge>
								) : (
									<Button
										variant="outline"
										size="sm"
										aria-label={`Log out ${label}`}
										isDisabled={ending}
										onPress={() => void end(session.id)}
									>
										Log out
									</Button>
								)}
							</li>
						);
					})}
				</ul>
			)}
		</BaseDialog>
	);
}
