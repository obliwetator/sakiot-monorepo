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
	Badge,
	Button,
	DialogHeading,
	IconButton,
	Modal,
	Notice,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Tooltip,
	TooltipTrigger,
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
		<Modal
			isOpen={open}
			onOpenChange={(isOpen) => {
				if (!isOpen) onClose();
			}}
		>
			<DialogHeading>
				{role ? (
					<>
						View as <em>{role.name}</em>
					</>
				) : (
					"View as role"
				)}
			</DialogHeading>
			<div className="space-y-3 px-5 py-4 border-y border-ui-border">
				{isLoading ? (
					<p className="leading-6">Loading preview…</p>
				) : isError ? (
					<p className="leading-6 text-danger">Failed to load preview.</p>
				) : (
					roleView && (
						<div className="flex flex-col gap-3">
							<div>
								<h6 className="leading-6 mb-2">What this role can see</h6>
								{roleView.can_manage_guild ? (
									<Badge tone={"warning"} size={"sm"}>
										Can manage the guild (admin pages included)
									</Badge>
								) : (
									<Badge appearance={"outline"} size={"sm"}>
										Cannot manage the guild
									</Badge>
								)}
							</div>
							{roleView.channels.length === 0 ? (
								<p className="text-muted text-sm">
									No voice channels in this guild.
								</p>
							) : (
								<div>
									<h6 className="leading-6 mb-2">
										Voice channels ({roleView.channels.length})
									</h6>
									<div className="flex flex-col gap-1">
										{roleView.channels.map((channel) => (
											<div
												key={channel.channel_id}
												className="relative flex items-center"
											>
												<Button
													className="w-full justify-start text-left"
													variant="ghost"
												>
													<div>
														{channel.name || channel.channel_id}
														<span className="block text-xs text-muted">
															{channel.channel_id}
														</span>
													</div>
												</Button>
												<div className="shrink-0 px-2">
													{channel.can_join ? (
														<Badge
															appearance={"outline"}
															tone={"success"}
															size={"sm"}
														>
															Can join
														</Badge>
													) : channel.can_view ? (
														<Badge appearance={"outline"} size={"sm"}>
															Visible only
														</Badge>
													) : (
														<Badge
															appearance={"outline"}
															tone={"danger"}
															size={"sm"}
														>
															Hidden
														</Badge>
													)}
												</div>
											</div>
										))}
									</div>
								</div>
							)}
							<Notice tone={"info"} announce="status">
								Open the audio preview to browse recordings, clips and stamps as
								this role. Channels marked "Visible only" or "Hidden" won't
								appear there — playback needs join permission, which the role
								lacks. Sessions spanning a hidden channel are invisible
								entirely.
							</Notice>
						</div>
					)
				)}
			</div>
			<div className="flex justify-end gap-2 border-t border-ui-border px-5 py-3">
				<Button onPress={onClose}>Close</Button>
				<Button variant="primary" isDisabled={!role} onPress={openAudioPreview}>
					<RemoveRedEyeIcon />
					Open audio preview
				</Button>
			</div>
		</Modal>
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

	if (!gid) return <div className="p-4">Missing guild id.</div>;

	return (
		<div className="p-4">
			<h5 className="font-semibold tracking-tight text-2xl mb-2">
				Members &amp; roles
			</h5>

			{loadingRoles ? (
				<p className="leading-6">Loading roles…</p>
			) : rolesError ? (
				<p className="leading-6 text-danger">
					Failed to load roles: {JSON.stringify(rolesErrorMessage)}
				</p>
			) : (
				<div className="flex items-stretch flex-col min-[900px]:flex-row gap-4">
					<div className="rounded-md border border-ui-border bg-surface text-fg shadow-none min-[900px]:min-w-70">
						<div className="flex flex-col gap-1">
							{(roles ?? []).map((role) => (
								<div key={role.role_id} className="relative flex items-center">
									<Button
										aria-pressed={selectedRole?.role_id === role.role_id}
										className="w-full justify-start text-left"
										variant="ghost"
										onPress={() => setSelectedRole(role)}
									>
										<span
											aria-hidden="true"
											className="w-3.5 h-3.5 rounded-sm shrink-0 mr-3"
											style={{ background: roleSwatchBackground(role) }}
										/>
										<div>
											{role.name}
											<span className="block text-xs text-muted">
												<Badge
													size={"sm"}
												>{`${role.member_count} member${role.member_count === 1 ? "" : "s"}`}</Badge>
											</span>
										</div>
									</Button>
									<div className="shrink-0 px-2">
										<TooltipTrigger delay={400}>
											<IconButton
												aria-label={`View as ${role.name}`}
												size="sm"
												onPress={() => setPreviewRole(role)}
											>
												<RemoveRedEyeIcon size={16} />
											</IconButton>
											<Tooltip>View server as this role</Tooltip>
										</TooltipTrigger>
									</div>
								</div>
							))}
							{(!roles || roles.length === 0) && (
								<div className="p-4">
									<p className="text-muted text-sm">
										No roles in this guild yet.
									</p>
								</div>
							)}
						</div>
					</div>

					<div className="w-full overflow-x-auto flex-1">
						{selectedRole && (
							<h6 className="text-base p-4">
								<span style={{ ...roleTextStyle(selectedRole) }}>
									{selectedRole.name}
								</span>{" "}
								— {members?.length ?? 0}{" "}
								{members?.length === 1 ? "member" : "members"}
							</h6>
						)}
						{loadingMembers ? (
							<p className="leading-6 p-4">Loading members…</p>
						) : membersError ? (
							<p className="leading-6 text-danger p-4">
								Failed to load members: {JSON.stringify(membersErrorMessage)}
							</p>
						) : (
							<Table>
								<TableHeader>
									<TableRow>
										<TableHead>Name</TableHead>
										<TableHead>User ID</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{(members ?? []).map((member) => (
										<TableRow key={member.user_id}>
											<TableCell>
												{member.name ?? <em>unknown user</em>}
											</TableCell>
											<TableCell>{member.user_id}</TableCell>
										</TableRow>
									))}
									{(!members || members.length === 0) && (
										<TableRow>
											<TableCell colSpan={2}>
												<p className="text-muted text-sm">
													{selectedRole
														? "No members hold this role."
														: "Select a role to see its members."}
												</p>
											</TableCell>
										</TableRow>
									)}
								</TableBody>
							</Table>
						)}
					</div>
				</div>
			)}

			<RolePreviewDialog
				open={previewRole !== null}
				guildId={gid}
				role={previewRole}
				onClose={() => setPreviewRole(null)}
			/>
		</div>
	);
}
