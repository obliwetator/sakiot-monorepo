use serde_json::json;
use serenity::model::guild::audit_log::AuditLogEntry;
use serenity::model::id::{ChannelId, GuildId, UserId};
use sqlx::PgPool;

use super::{
    AUDIT_WINDOW_SECONDS, Attribution, OwnVoice, OwnVoiceEvent, classify, find_actor, record,
};

const BOT: UserId = UserId::new(500);
const MODERATOR: UserId = UserId::new(600);
const GENERAL: ChannelId = ChannelId::new(10);
const MUSIC: ChannelId = ChannelId::new(20);
/// An arbitrary moment, in Unix seconds.
const NOW_SECONDS: i64 = 1_794_000_000;
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;

fn in_voice(channel: ChannelId, deaf: bool) -> OwnVoice {
    OwnVoice {
        channel_id: Some(channel),
        deaf,
    }
}

fn out_of_voice() -> OwnVoice {
    OwnVoice {
        channel_id: None,
        deaf: false,
    }
}

#[test]
fn someone_else_deafening_the_bot_is_an_event() {
    assert_eq!(
        classify(
            in_voice(GENERAL, false),
            in_voice(GENERAL, true),
            Some(GENERAL)
        ),
        [OwnVoiceEvent::Deafened { channel: GENERAL }]
    );
}

#[test]
fn staying_deafened_is_not_a_new_event() {
    assert!(
        classify(
            in_voice(GENERAL, true),
            in_voice(GENERAL, true),
            Some(GENERAL)
        )
        .is_empty()
    );
}

#[test]
fn being_undeafened_ends_the_span() {
    assert_eq!(
        classify(
            in_voice(GENERAL, true),
            in_voice(GENERAL, false),
            Some(GENERAL)
        ),
        [OwnVoiceEvent::Undeafened { channel: GENERAL }]
    );
}

#[test]
fn a_disconnect_while_deafened_is_no_undeafen() {
    // The deafen outlives the visit: Discord still reports it.
    let gone = OwnVoice {
        channel_id: None,
        deaf: true,
    };
    assert_eq!(
        classify(in_voice(GENERAL, true), gone, Some(GENERAL)),
        [OwnVoiceEvent::Disconnected { from: GENERAL }]
    );
}

#[test]
fn a_switch_the_bot_asked_for_is_not_an_event() {
    assert!(
        classify(
            in_voice(GENERAL, false),
            in_voice(MUSIC, false),
            Some(MUSIC)
        )
        .is_empty()
    );
}

#[test]
fn a_move_the_bot_did_not_ask_for_is_an_event() {
    assert_eq!(
        classify(
            in_voice(GENERAL, false),
            in_voice(MUSIC, false),
            Some(GENERAL)
        ),
        [OwnVoiceEvent::Moved {
            from: GENERAL,
            to: MUSIC
        }]
    );
}

#[test]
fn a_leave_the_bot_asked_for_is_not_an_event() {
    assert!(classify(in_voice(GENERAL, false), out_of_voice(), None).is_empty());
}

#[test]
fn a_disconnect_the_bot_did_not_ask_for_is_an_event() {
    assert_eq!(
        classify(in_voice(GENERAL, false), out_of_voice(), Some(GENERAL)),
        [OwnVoiceEvent::Disconnected { from: GENERAL }]
    );
}

#[test]
fn joining_clean_is_not_an_event() {
    assert!(classify(out_of_voice(), in_voice(GENERAL, false), Some(GENERAL)).is_empty());
}

#[test]
fn joining_deafened_asks_for_a_cleanup_not_a_culprit() {
    assert_eq!(
        classify(out_of_voice(), in_voice(GENERAL, true), Some(GENERAL)),
        [OwnVoiceEvent::JoinedDeafened { channel: GENERAL }]
    );
}

#[test]
fn moving_while_deafened_keeps_the_deafen() {
    // Only a full disconnect clears it, so this asks for no cleanup.
    assert_eq!(
        classify(
            in_voice(GENERAL, true),
            in_voice(MUSIC, true),
            Some(GENERAL)
        ),
        [OwnVoiceEvent::Moved {
            from: GENERAL,
            to: MUSIC
        }]
    );
}

#[test]
fn deafened_and_moved_at_once_is_both() {
    assert_eq!(
        classify(
            in_voice(GENERAL, false),
            in_voice(MUSIC, true),
            Some(GENERAL)
        ),
        [
            OwnVoiceEvent::Moved {
                from: GENERAL,
                to: MUSIC
            },
            OwnVoiceEvent::Deafened { channel: MUSIC }
        ]
    );
}

/// An audit entry `age_seconds` old, its id carrying that time as Discord's do.
fn entry(age_seconds: i64, extra: serde_json::Value) -> Result<AuditLogEntry, serde_json::Error> {
    let at_ms = (NOW_SECONDS - age_seconds) * 1000;
    let snowflake = (at_ms - DISCORD_EPOCH_MS) << 22;
    let mut value = json!({
        "id": snowflake.to_string(),
        "user_id": MODERATOR.get().to_string(),
        "target_id": null,
        "action_type": 27,
    });
    if let (Some(value), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        value.extend(extra.clone());
    }
    serde_json::from_value(value)
}

fn deafen_of(target: UserId) -> serde_json::Value {
    json!({
        "action_type": 24,
        "target_id": target.get().to_string(),
        "changes": [{"key": "deaf", "old_value": false, "new_value": true}],
    })
}

type TestResult = Result<(), serde_json::Error>;

#[test]
fn a_deafen_of_the_bot_names_its_actor_exactly() -> TestResult {
    let entries = [entry(3, deafen_of(BOT))?];
    let event = OwnVoiceEvent::Deafened { channel: GENERAL };
    assert_eq!(
        find_actor(&entries, event, BOT, NOW_SECONDS),
        Some(Attribution::Exact(MODERATOR))
    );
    Ok(())
}

#[test]
fn a_deafen_of_someone_else_is_not_the_bots() -> TestResult {
    let entries = [entry(3, deafen_of(UserId::new(700)))?];
    let event = OwnVoiceEvent::Deafened { channel: GENERAL };
    assert_eq!(find_actor(&entries, event, BOT, NOW_SECONDS), None);
    Ok(())
}

#[test]
fn an_undeafen_does_not_explain_a_deafen() -> TestResult {
    let undeafen = json!({
        "action_type": 24,
        "target_id": BOT.get().to_string(),
        "changes": [{"key": "deaf", "old_value": true, "new_value": false}],
    });
    let entries = [entry(3, undeafen)?];
    let event = OwnVoiceEvent::Deafened { channel: GENERAL };
    assert_eq!(find_actor(&entries, event, BOT, NOW_SECONDS), None);
    Ok(())
}

fn undeafen_by(actor: UserId) -> serde_json::Value {
    json!({
        "action_type": 24,
        "user_id": actor.get().to_string(),
        "target_id": BOT.get().to_string(),
        "changes": [{"key": "deaf", "old_value": true, "new_value": false}],
    })
}

#[test]
fn an_undeafen_names_who_did_it() -> TestResult {
    let entries = [entry(3, undeafen_by(MODERATOR))?];
    let event = OwnVoiceEvent::Undeafened { channel: GENERAL };
    assert_eq!(
        find_actor(&entries, event, BOT, NOW_SECONDS),
        Some(Attribution::Exact(MODERATOR))
    );
    Ok(())
}

#[test]
fn clearing_a_leftover_deafen_names_the_bot() -> TestResult {
    let entries = [entry(3, undeafen_by(BOT))?];
    let event = OwnVoiceEvent::Undeafened { channel: GENERAL };
    assert_eq!(
        find_actor(&entries, event, BOT, NOW_SECONDS),
        Some(Attribution::Exact(BOT))
    );
    Ok(())
}

#[test]
fn a_deafen_does_not_explain_an_undeafen() -> TestResult {
    let entries = [entry(3, deafen_of(BOT))?];
    let event = OwnVoiceEvent::Undeafened { channel: GENERAL };
    assert_eq!(find_actor(&entries, event, BOT, NOW_SECONDS), None);
    Ok(())
}

#[test]
fn a_disconnect_entry_without_a_target_is_only_likely() -> TestResult {
    let entries = [entry(3, json!({"options": {"count": "1"}}))?];
    let event = OwnVoiceEvent::Disconnected { from: GENERAL };
    assert_eq!(
        find_actor(&entries, event, BOT, NOW_SECONDS),
        Some(Attribution::Likely(MODERATOR))
    );
    Ok(())
}

#[test]
fn a_disconnect_naming_the_bot_is_exact() -> TestResult {
    let entries = [entry(3, json!({"target_id": BOT.get().to_string()}))?];
    let event = OwnVoiceEvent::Disconnected { from: GENERAL };
    assert_eq!(
        find_actor(&entries, event, BOT, NOW_SECONDS),
        Some(Attribution::Exact(MODERATOR))
    );
    Ok(())
}

#[test]
fn a_disconnect_naming_someone_else_is_skipped() -> TestResult {
    let entries = [entry(3, json!({"target_id": "700"}))?];
    let event = OwnVoiceEvent::Disconnected { from: GENERAL };
    assert_eq!(find_actor(&entries, event, BOT, NOW_SECONDS), None);
    Ok(())
}

#[test]
fn a_move_counts_only_into_the_channel_the_bot_landed_in() -> TestResult {
    let into = |channel: ChannelId| {
        json!({
            "action_type": 26,
            "options": {"channel_id": channel.get().to_string(), "count": "1"},
        })
    };
    let event = OwnVoiceEvent::Moved {
        from: GENERAL,
        to: MUSIC,
    };
    assert_eq!(
        find_actor(&[entry(3, into(GENERAL))?], event, BOT, NOW_SECONDS),
        None
    );
    assert_eq!(
        find_actor(&[entry(3, into(MUSIC))?], event, BOT, NOW_SECONDS),
        Some(Attribution::Likely(MODERATOR))
    );
    Ok(())
}

#[test]
fn an_entry_past_the_window_names_nobody() -> TestResult {
    let event = OwnVoiceEvent::Disconnected { from: GENERAL };
    let fresh = [entry(AUDIT_WINDOW_SECONDS, json!({}))?];
    let stale = [entry(AUDIT_WINDOW_SECONDS + 1, json!({}))?];
    assert!(find_actor(&fresh, event, BOT, NOW_SECONDS).is_some());
    assert_eq!(find_actor(&stale, event, BOT, NOW_SECONDS), None);
    Ok(())
}

#[test]
fn the_newest_match_wins() -> TestResult {
    let newer = json!({"user_id": "601"});
    let entries = [entry(3, newer)?, entry(30, json!({}))?];
    let event = OwnVoiceEvent::Disconnected { from: GENERAL };
    assert_eq!(
        find_actor(&entries, event, BOT, NOW_SECONDS),
        Some(Attribution::Likely(UserId::new(601)))
    );
    Ok(())
}

#[test]
fn the_bot_is_never_its_own_culprit() -> TestResult {
    let by_bot = json!({"user_id": BOT.get().to_string()});
    let entries = [entry(3, by_bot)?];
    let event = OwnVoiceEvent::Disconnected { from: GENERAL };
    assert_eq!(find_actor(&entries, event, BOT, NOW_SECONDS), None);
    Ok(())
}

/// The database constrains every label the bot writes.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn every_row_the_bot_writes_is_accepted(pool: PgPool) -> Result<(), sqlx::Error> {
    let guild = GuildId::new(1);
    let rows = [
        (("deafened", GENERAL, None), Attribution::Exact(MODERATOR)),
        (
            ("moved", GENERAL, Some(MUSIC)),
            Attribution::Likely(MODERATOR),
        ),
        (("disconnected", GENERAL, None), Attribution::Unknown),
        (("disconnected", GENERAL, None), Attribution::NoAccess),
        (("undeafened", GENERAL, None), Attribution::Exact(BOT)),
        (
            ("deafen_clear_failed", GENERAL, None),
            Attribution::Exact(BOT),
        ),
    ];
    for (row, attribution) in rows {
        record(&pool, guild, row, attribution, NOW_SECONDS * 1000).await;
    }
    let written: i64 = sqlx::query_scalar("SELECT count(*) FROM bot_voice_log")
        .fetch_one(&pool)
        .await?;
    assert_eq!(written, 6);
    Ok(())
}
