import { Eye as RemoveRedEyeIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import type { GuildRole } from "../../app/apiSlice";
import {
	useGetGuildRolesQuery,
	useGetRoleMembersQuery,
	useGetRoleViewQuery,
} from "../../app/apiSlice";
import { PATH_PREFIX_FOR_LOGGED_USERS } from "../../Constants";
import {
	Button,
	cn,
	Dialog,
	DialogActions,
	DialogContent,
	DialogTitle,
	IconButton,
	Notice,
	Page,
	PageTitle,
	Panel,
	SectionTitle,
	Table,
	TableBody,
	TableCell,
	TableContainer,
	TableHead,
	TableHeader,
	TableRow,
	Text,
} from "../../shared/ui";
import { roleSwatchBackground, roleTextStyle } from "./roleColors";

function RolePreviewDialog(props: {
	open: boolean;
	guildId: string;
	role: GuildRole | null;
	onClose: () => void;
}) {
	const { open, guildId, role, onClose } = props;
	const navigate = useNavigate();
	const {
		data: roleView,
		isLoading,
		isError,
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
		<Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
			<DialogTitle>
				{role ? (
					<>
						View as <em>{role.name}</em>
					</>
				) : (
					"View as role"
				)}
			</DialogTitle>
			<DialogContent dividers>
				{isLoading ? (
					<Text aria-live="polite">Loading preview…</Text>
				) : isError ? (
					<Notice tone="error" announce="alert">
						Failed to load preview.
					</Notice>
				) : (
					roleView && (
						<div className="space-y-4">
							<div>
								<div className="mb-1 text-xs font-semibold text-slate-200">
									What this role can see
								</div>
								{roleView.can_manage_guild ? (
									<span className="inline-flex rounded-full border border-amber-500/40 bg-amber-500/10 px-2.5 py-0.5 text-xs font-medium text-amber-200">
										Can manage the guild (admin pages included)
									</span>
								) : (
									<span className="inline-flex rounded-full border border-ui-border bg-surface-raised px-2.5 py-0.5 text-xs font-medium text-muted">
										Cannot manage the guild
									</span>
								)}
							</div>
							{roleView.channels.length === 0 ? (
								<Text tone="muted">No voice channels in this guild.</Text>
							) : (
								<div>
									<div className="mb-2 text-xs font-semibold text-slate-200">
										Voice channels ({roleView.channels.length})
									</div>
									<div className="max-h-60 divide-y divide-ui-border/40 overflow-y-auto rounded-md border border-ui-border bg-slate-950/40">
										{roleView.channels.map((channel) => (
											<div
												key={channel.channel_id}
												className="flex items-center justify-between gap-2 px-3 py-2 text-sm"
											>
												<div className="min-w-0">
													<div className="truncate font-medium text-fg">
														{channel.name || channel.channel_id}
													</div>
													<div className="truncate font-mono text-xs text-muted">
														{channel.channel_id}
													</div>
												</div>
												<span
													className={cn(
														"shrink-0 rounded-full border px-2 py-0.5 text-xs font-medium",
														channel.can_join
															? "border-emerald-500/40 bg-emerald-500/10 text-emerald-200"
															: channel.can_view
																? "border-ui-border bg-surface-raised text-muted"
																: "border-red-500/40 bg-red-500/10 text-red-200",
													)}
												>
													{channel.can_join
														? "Can join"
														: channel.can_view
															? "Visible only"
															: "Hidden"}
												</span>
											</div>
										))}
									</div>
								</div>
							)}
							<Notice tone="info">
								Open the audio preview to browse recordings, clips and stamps as
								this role. Channels marked "Visible only" or "Hidden" won't
								appear there — playback needs join permission, which the role
								lacks. Sessions spanning a hidden channel are invisible
								entirely.
							</Notice>
						</div>
					)
				)}
			</DialogContent>
			<DialogActions>
				<Button variant="secondary" onClick={onClose}>
					Close
				</Button>
				<Button
					variant="primary"
					startIcon={<RemoveRedEyeIcon aria-hidden="true" className="size-4" />}
					onClick={openAudioPreview}
					disabled={!role}
				>
					Open audio preview
				</Button>
			</DialogActions>
		</Dialog>
	);
}

export function GuildMembers() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const gid = guild_id ?? "";

	const {
		data: roles,
		isLoading: loadingRoles,
		isError: rolesError,
		error: rolesErrorMessage,
	} = useGetGuildRolesQuery(gid, { skip: !gid });
	const [selectedRole, setSelectedRole] = useState<GuildRole | null>(null);
	const [previewRole, setPreviewRole] = useState<GuildRole | null>(null);

	useEffect(() => {
		const stillListed = (roles ?? []).some(
			(role) => role.role_id === selectedRole?.role_id,
		);
		if (!stillListed) setSelectedRole(roles?.[0] ?? null);
	}, [roles, selectedRole]);

	const {
		data: members,
		isLoading: loadingMembers,
		isError: membersError,
		error: membersErrorMessage,
	} = useGetRoleMembersQuery(
		{ guild_id: gid, role_id: selectedRole?.role_id ?? "" },
		{ skip: !gid || !selectedRole },
	);

	if (!gid) {
		return (
			<Page>
				<Notice tone="error">Missing guild id.</Notice>
			</Page>
		);
	}

	return (
		<Page className="space-y-5">
			<header className="space-y-1">
				<PageTitle>Members &amp; roles</PageTitle>
				<Text tone="muted">
					Inspect server roles, check permissions, and view role members.
				</Text>
			</header>

			{loadingRoles ? (
				<Text aria-live="polite">Loading roles…</Text>
			) : rolesError ? (
				<Notice tone="error" announce="alert">
					Failed to load roles: {JSON.stringify(rolesErrorMessage)}
				</Notice>
			) : (
				<div className="flex flex-col items-stretch gap-4 md:flex-row">
					<Panel className="p-0 overflow-hidden md:w-80 shrink-0">
						<div className="border-b border-ui-border px-4 py-3 font-semibold text-fg">
							Roles ({roles?.length ?? 0})
						</div>
						<div className="divide-y divide-ui-border/40">
							{(roles ?? []).map((role) => {
								const isSelected = selectedRole?.role_id === role.role_id;
								return (
									<div
										key={role.role_id}
										className={cn(
											"flex items-center justify-between gap-2 px-3 py-2.5 transition-colors",
											isSelected ? "bg-slate-800/80" : "hover:bg-slate-800/40",
										)}
									>
										<button
											type="button"
											onClick={() => setSelectedRole(role)}
											className="flex min-w-0 flex-1 items-center gap-2.5 text-left"
										>
											<span
												aria-hidden="true"
												className="size-3.5 shrink-0 rounded-xs"
												style={{ background: roleSwatchBackground(role) }}
											/>
											<span className="truncate text-sm font-medium text-fg">
												{role.name}
											</span>
											<span className="ml-auto shrink-0 rounded-full border border-ui-border bg-surface-raised px-2 py-0.5 text-xs text-muted">
												{role.member_count}
											</span>
										</button>
										<IconButton
											label={`View as ${role.name}`}
											variant="ghost"
											size="sm"
											onPress={() => setPreviewRole(role)}
										>
											<RemoveRedEyeIcon
												aria-hidden="true"
												className="size-4 text-muted hover:text-fg"
											/>
										</IconButton>
									</div>
								);
							})}
							{(!roles || roles.length === 0) && (
								<div className="p-4 text-sm text-muted">
									No roles in this guild yet.
								</div>
							)}
						</div>
					</Panel>

					<Panel className="flex-1 space-y-4">
						{selectedRole && (
							<SectionTitle>
								<span style={roleTextStyle(selectedRole)}>
									{selectedRole.name}
								</span>{" "}
								<span className="text-sm font-normal text-muted">
									— {members?.length ?? 0}{" "}
									{members?.length === 1 ? "member" : "members"}
								</span>
							</SectionTitle>
						)}

						{loadingMembers ? (
							<Text aria-live="polite">Loading members…</Text>
						) : membersError ? (
							<Notice tone="error" announce="alert">
								Failed to load members: {JSON.stringify(membersErrorMessage)}
							</Notice>
						) : (
							<TableContainer className="rounded-lg border border-slate-800">
								<Table>
									<caption className="sr-only">
										Members of {selectedRole?.name}
									</caption>
									<TableHeader>
										<TableRow>
											<TableHead scope="col">Name</TableHead>
											<TableHead scope="col">User ID</TableHead>
										</TableRow>
									</TableHeader>
									<TableBody>
										{(members ?? []).map((member) => (
											<TableRow key={member.user_id}>
												<TableCell className="font-medium text-fg">
													{member.name ?? (
														<span className="italic text-muted">
															unknown user
														</span>
													)}
												</TableCell>
												<TableCell className="font-mono text-xs text-cyan-100">
													{member.user_id}
												</TableCell>
											</TableRow>
										))}
										{(!members || members.length === 0) && (
											<TableRow>
												<TableCell
													colSpan={2}
													className="py-8 text-center text-muted"
												>
													{selectedRole
														? "No members hold this role."
														: "Select a role to see its members."}
												</TableCell>
											</TableRow>
										)}
									</TableBody>
								</Table>
							</TableContainer>
						)}
					</Panel>
				</div>
			)}

			<RolePreviewDialog
				open={previewRole !== null}
				guildId={gid}
				role={previewRole}
				onClose={() => setPreviewRole(null)}
			/>
		</Page>
	);
}
