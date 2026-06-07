use serenity::{
    client::Context,
    model::id::{ChannelId, GuildId},
};
use songbird::events::context_data::DisconnectReason;

pub(super) const RECOVERABLE_DISCONNECT_TIMEOUT_MS: u64 = 60_000;

// Without this the only way to test Discord's recoverable disconnect path is
// waiting for a random driver disconnect. This intentionally stays false in prod.
pub(super) const RESUME_INTENTIONAL_DISCONNECTS_FOR_TESTING: bool = false;

pub(super) fn is_intentional_driver_disconnect(reason: Option<&DisconnectReason>) -> bool {
    reason.is_none() || matches!(reason, Some(DisconnectReason::Requested))
}

pub(super) fn should_resume_recordings_for_disconnect(
    reason: Option<&DisconnectReason>,
    resume_intentional_disconnects: bool,
) -> bool {
    resume_intentional_disconnects || !is_intentional_driver_disconnect(reason)
}

pub(super) fn should_finalize_empty_channel_disconnect(
    reason: Option<&DisconnectReason>,
    channel_has_human_members: Option<bool>,
) -> bool {
    is_intentional_driver_disconnect(reason) && channel_has_human_members == Some(false)
}

pub(super) fn recording_channel_has_human_members(
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Option<bool> {
    let guild = ctx.cache.guild(guild_id)?;
    let bot_id = ctx.cache.current_user().id;

    for (user_id, voice_state) in &guild.voice_states {
        if voice_state.channel_id != Some(channel_id) {
            continue;
        }

        if *user_id == bot_id {
            continue;
        }

        let Some(member) = guild.members.get(user_id) else {
            return Some(true);
        };

        if !member.user.bot {
            return Some(true);
        }
    }

    Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_and_requested_disconnects_are_intentional() {
        assert!(is_intentional_driver_disconnect(None));
        assert!(is_intentional_driver_disconnect(Some(
            &DisconnectReason::Requested
        )));
        assert!(!is_intentional_driver_disconnect(Some(
            &DisconnectReason::TimedOut
        )));
    }

    #[test]
    fn optional_testing_flag_can_resume_intentional_disconnects() {
        assert!(!should_resume_recordings_for_disconnect(None, false));
        assert!(should_resume_recordings_for_disconnect(None, true));
        assert!(should_resume_recordings_for_disconnect(
            Some(&DisconnectReason::TimedOut),
            false
        ));
    }

    #[test]
    fn production_default_does_not_resume_requested_disconnects() {
        assert!(!should_resume_recordings_for_disconnect(
            Some(&DisconnectReason::Requested),
            RESUME_INTENTIONAL_DISCONNECTS_FOR_TESTING
        ));
    }

    #[test]
    fn empty_channel_intentional_disconnects_finalize_even_when_resume_testing_enabled() {
        assert!(should_finalize_empty_channel_disconnect(None, Some(false)));
        assert!(should_finalize_empty_channel_disconnect(
            Some(&DisconnectReason::Requested),
            Some(false)
        ));
        assert!(!should_finalize_empty_channel_disconnect(
            Some(&DisconnectReason::Requested),
            Some(true)
        ));
        assert!(!should_finalize_empty_channel_disconnect(
            Some(&DisconnectReason::Requested),
            None
        ));
        assert!(!should_finalize_empty_channel_disconnect(
            Some(&DisconnectReason::TimedOut),
            Some(false)
        ));
    }
}
