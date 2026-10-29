import { Eye as RemoveRedEyeIcon } from "lucide-react";
import { useNavigate } from "react-router-dom";
import type { CapabilityDecision, GuildRole } from "../../app/apiSlice";
import { useGetRoleViewQuery } from "../../app/apiSlice";
import { PATH_PREFIX_FOR_LOGGED_USERS } from "../../Constants";
import { BaseDialog } from "../../shared/BaseDialog";
import { LoadFailure } from "../../shared/LoadFailure";
import { Badge, cn, Notice } from "../../shared/ui";
import { Held, SourceMark } from "../admin-permissions/Decision";
import { CAPABILITIES } from "../admin-permissions/rules";
import { type ChannelStatus, channelStatus, reasons } from "./rolePreview";

/**
 * What a member holding only `role` may do, capability by capability, with
 * Sakiot's rules on top of Discord's, and why wherever Discord alone did
 * not decide.
 */
export function RolePreviewDialog(props: {
	open: boolean;
	guildId: string;
	role: GuildRole | null;
	roles: GuildRole[];
	onClose: () => void;
}) {
	const { open, guildId, role, roles, onClose } = props;
	const navigate = useNavigate();
	const {
		data: roleView,
		isLoading,
		isError,
		error,
		refetch,
	} = useGetRoleViewQuery(
		{ guild_id: guildId, role_id: role?.role_id ?? "" },
		{ skip: !open || !role },
	);

	const openAudioPreview = () => {
		if (!role) return;
		onClose();
		navigate(
			`${PATH_PREFIX_FOR_LOGGED_USERS}/${guildId}/audio?as_role=${role.role_id}`,
		);
	};

	return (
		<BaseDialog
			open={open}
			onClose={onClose}
			title={
				role ? (
					<>
						View as <em>{role.name}</em>
					</>
				) : (
					"View as role"
				)
			}
			cancelLabel="Close"
			confirmLabel={
				<>
					<RemoveRedEyeIcon />
					Open audio preview
				</>
			}
			confirmDisabled={!role}
			onConfirm={openAudioPreview}
		>
			{isLoading ? (
				<p className="leading-6">Loading preview…</p>
			) : isError ? (
				<LoadFailure
					error={error}
					what="Could not load the role preview."
					onRetry={() => void refetch()}
				/>
			) : (
				roleView && (
					<div className="flex flex-col gap-4">
						<p className="text-sm text-muted">
							Discord's permissions, then Sakiot's own rules. Anything Discord
							alone did not decide says why.
						</p>
						<section
							aria-labelledby="role-preview-server"
							className="space-y-2"
						>
							<h6 id="role-preview-server" className="leading-6">
								Server-wide
							</h6>
							<Decisions decisions={roleView.guild} />
							<Reasons
								decisions={roleView.guild}
								roles={roles}
								channel={null}
							/>
						</section>
						{roleView.channels.length === 0 ? (
							<p className="text-muted text-sm">
								No voice channels in this guild.
							</p>
						) : (
							<section
								aria-labelledby="role-preview-channels"
								className="space-y-2"
							>
								<h6 id="role-preview-channels" className="leading-6">
									Voice channels ({roleView.channels.length})
								</h6>
								<ul className="flex flex-col gap-2">
									{roleView.channels.map((channel) => (
										<li
											key={channel.channel_id}
											aria-label={channel.name || channel.channel_id}
											className="space-y-2 rounded-md border border-ui-border p-3"
										>
											<div className="flex items-start justify-between gap-2">
												<div className="min-w-0">
													{channel.name || channel.channel_id}
													<span className="block text-xs text-muted">
														{channel.channel_id}
													</span>
												</div>
												<StatusBadge
													status={channelStatus(channel.capabilities)}
												/>
											</div>
											<Decisions decisions={channel.capabilities} />
											<Reasons
												decisions={channel.capabilities}
												roles={roles}
												channel={channel.channel_id}
											/>
										</li>
									))}
								</ul>
							</section>
						)}
						<Notice tone={"info"} announce="status">
							Open the audio preview to browse recordings, clips and stamps as
							this role, Sakiot's rules included. Recordings it can't listen to
							stay listed but marked; its clips and stamps from those channels
							don't appear.
						</Notice>
					</div>
				)
			)}
		</BaseDialog>
	);
}

const STATUS: Record<
	ChannelStatus,
	{ label: string; tone: "success" | "neutral" | "danger" }
> = {
	listens: { label: "Can listen", tone: "success" },
	sees: { label: "Can't listen", tone: "neutral" },
	hidden: { label: "Hidden", tone: "danger" },
};

function StatusBadge({ status }: { status: ChannelStatus }) {
	const { label, tone } = STATUS[status];
	return (
		<Badge appearance={"outline"} tone={tone} size={"sm"}>
			{label}
		</Badge>
	);
}

const MARK_DESCRIPTION: Record<CapabilityDecision["source"]["kind"], string> = {
	discord: "decided by Discord",
	owner: "kept by the owner",
	rule: "decided by a Sakiot rule",
	unseen: "taken by not seeing the channel",
	manager: "left out: you don't hold it yourself",
	exempt: "kept: rules take nothing from whoever can edit permissions",
};

function Decisions({ decisions }: { decisions: CapabilityDecision[] }) {
	return (
		<ul className="flex flex-wrap gap-1.5">
			{decisions.map((decision) => (
				<li
					key={decision.capability}
					className="flex items-center gap-1 rounded-md border border-ui-border px-1.5 py-0.5 text-xs"
				>
					<Held decision={decision} />
					{CAPABILITIES[decision.capability].label}
					<SourceMark
						decision={decision}
						description={MARK_DESCRIPTION[decision.source.kind]}
					/>
				</li>
			))}
		</ul>
	);
}

function Reasons({
	decisions,
	roles,
	channel,
}: {
	decisions: CapabilityDecision[];
	roles: GuildRole[];
	channel: string | null;
}) {
	const lines = reasons(decisions, roles, channel);
	if (lines.length === 0) return null;
	return (
		<ul className="space-y-0.5 text-xs">
			{lines.map((reason) => (
				<li
					key={reason.text}
					className={cn(reason.held ? "text-emerald-200" : "text-rose-200")}
				>
					{reason.text}
				</li>
			))}
		</ul>
	);
}
