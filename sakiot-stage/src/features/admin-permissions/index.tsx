import { useParams } from "react-router-dom";
import { useGetPermissionRulesQuery } from "../../app/apiSlice";
import { LoadFailure } from "../../shared/LoadFailure";
import {
	Notice,
	Page,
	PageTitle,
	Panel,
	SectionTitle,
	Text,
} from "../../shared/ui";
import { AccessExplainer } from "./AccessExplainer";
import { RuleEditor, StaleRules } from "./RuleEditor";

/**
 * Sakiot's own permission rules on top of Discord's, for holders of
 * `manage_permissions`, and what they add up to for a member or a role.
 */
export function GuildPermissionsPage() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const gid = guild_id ?? "";
	const rulesQuery = useGetPermissionRulesQuery(gid, { skip: !gid });
	const data = rulesQuery.data;

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
				<PageTitle>Permissions</PageTitle>
				<Text tone="muted">
					Everyone starts with what their Discord permissions give them. A rule
					here allows or denies more, to @everyone, a role or a member,
					server-wide or in one voice channel. The most specific rule wins:
					server-wide @everyone, then roles, then the member; then the same in
					the channel. The owner always keeps managing the server, and nobody
					hears a member who keeps their recordings private.
				</Text>
			</header>

			{rulesQuery.isLoading && <Text aria-live="polite">Loading rules…</Text>}
			{rulesQuery.isError && (
				<LoadFailure
					error={rulesQuery.error}
					what="Could not load the permission rules."
					onRetry={() => void rulesQuery.refetch()}
				/>
			)}

			{data && (
				<>
					<Panel aria-labelledby="rules-heading" className="space-y-4">
						<SectionTitle id="rules-heading">Rules</SectionTitle>
						<RuleEditor guildId={gid} data={data} />
						<StaleRules guildId={gid} data={data} />
					</Panel>
					<Panel aria-labelledby="effective-heading" className="space-y-4">
						<div className="space-y-1">
							<SectionTitle id="effective-heading">
								Effective access
							</SectionTitle>
							<Text tone="muted">
								What a role or a member holds once Discord and these rules have
								had their say.
							</Text>
						</div>
						<AccessExplainer guildId={gid} data={data} everyoneId={gid} />
					</Panel>
				</>
			)}
		</Page>
	);
}
