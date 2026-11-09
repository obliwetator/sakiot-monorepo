use super::{
    Action, CONFIRM_BUTTON_PREFIX, MAX_CHOICE_NAME, MAX_CHOICES, Resolved, Target, UserGuild,
    apply, choice_label, confirmation_prompt, needs_confirmation, parse_confirmation, rank_guilds,
    resolve_target, with_former,
};
use crate::database::logical_recordings::create_fragment_in;
use crate::database::recordings::RecordingOwner;
use crate::database::{DbError, opt_outs, privacy};
use sqlx::PgPool;

const ALL_CHANGES: [Action; 4] = [
    Action::OptOut,
    Action::OptIn,
    Action::Private,
    Action::Shared,
];

fn guild(id: i64, name: &str) -> UserGuild {
    UserGuild {
        id,
        name: name.to_string(),
        former: false,
    }
}

fn left(id: i64, name: &str) -> UserGuild {
    UserGuild {
        former: true,
        ..guild(id, name)
    }
}

fn guilds() -> Vec<UserGuild> {
    vec![guild(1, "Rouvas"), guild(2, "Friends"), guild(3, "Work")]
}

fn other(guild_id: i64, name: &str) -> Resolved {
    Resolved::Guild(Target {
        guild_id,
        name: Some(name.to_string()),
        former: false,
    })
}

fn this_server(guild_id: i64) -> Resolved {
    Resolved::Guild(Target {
        guild_id,
        name: None,
        former: false,
    })
}

#[test]
fn an_empty_field_in_a_server_means_that_server() {
    assert_eq!(resolve_target(&guilds(), Some(1), None), this_server(1));
    assert_eq!(
        resolve_target(&guilds(), Some(1), Some("  ")),
        this_server(1)
    );
}

#[test]
fn an_empty_field_in_a_direct_message_asks_which_server() {
    assert_eq!(resolve_target(&guilds(), None, None), Resolved::NeedsServer);
}

#[test]
fn a_picked_server_is_found_by_its_id() {
    assert_eq!(
        resolve_target(&guilds(), Some(1), Some("2")),
        other(2, "Friends")
    );
    assert_eq!(
        resolve_target(&guilds(), None, Some("2")),
        other(2, "Friends")
    );
}

#[test]
fn a_typed_name_matches_whatever_its_case() {
    assert_eq!(
        resolve_target(&guilds(), None, Some("friends")),
        other(2, "Friends")
    );
}

/// Choosing the server you're already in is not "another server".
#[test]
fn naming_the_current_server_is_still_the_current_server() {
    assert_eq!(
        resolve_target(&guilds(), Some(2), Some("2")),
        this_server(2)
    );
    assert_eq!(
        resolve_target(&guilds(), Some(2), Some("Friends")),
        this_server(2)
    );
}

/// The field is free text: a member can type any server's id.
#[test]
fn a_server_the_user_is_not_in_is_unknown() {
    assert_eq!(
        resolve_target(&guilds(), Some(1), Some("999")),
        Resolved::Unknown
    );
    assert_eq!(
        resolve_target(&guilds(), None, Some("Nope")),
        Resolved::Unknown
    );
}

#[test]
fn two_servers_with_one_name_ask_for_a_pick() {
    let guilds = vec![guild(1, "Same"), guild(2, "same")];
    assert_eq!(
        resolve_target(&guilds, None, Some("Same")),
        Resolved::Ambiguous
    );
    assert_eq!(resolve_target(&guilds, None, Some("2")), other(2, "same"));
}

#[test]
fn a_server_someone_left_resolves_as_former() {
    let guilds = vec![left(5, "Old")];
    assert_eq!(
        resolve_target(&guilds, None, Some("5")),
        Resolved::Guild(Target {
            guild_id: 5,
            name: Some("Old".to_string()),
            former: true,
        })
    );
}

/// Typing in a server means being in it, even if the cache hasn't caught up.
#[test]
fn the_server_being_typed_in_is_never_former() {
    let guilds = vec![left(5, "Old")];
    assert_eq!(resolve_target(&guilds, Some(5), Some("5")), this_server(5));
}

#[test]
fn only_who_hears_it_stays_open_after_leaving() {
    assert!(!Action::OptOut.open_to_former());
    assert!(!Action::OptIn.open_to_former());
    assert!(Action::Private.open_to_former());
    assert!(Action::Shared.open_to_former());
    assert!(Action::Status.open_to_former());
}

#[test]
fn a_recorded_server_is_former_only_if_the_user_is_not_in_it() {
    let merged = with_former(
        vec![guild(1, "Rouvas")],
        vec![(1, "Rouvas".to_string()), (5, "Old".to_string())],
    );
    assert_eq!(merged, vec![guild(1, "Rouvas"), left(5, "Old")]);
}

#[test]
fn only_a_change_to_another_server_is_confirmed() {
    let elsewhere = Target {
        guild_id: 2,
        name: Some("Friends".to_string()),
        former: false,
    };
    let here = Target {
        guild_id: 1,
        name: None,
        former: false,
    };
    for action in ALL_CHANGES {
        assert!(needs_confirmation(action, &elsewhere), "{action:?}");
        assert!(!needs_confirmation(action, &here), "{action:?}");
    }
    assert!(!needs_confirmation(Action::Status, &elsewhere));
}

#[test]
fn each_change_asks_about_the_named_server() {
    for action in ALL_CHANGES {
        assert!(confirmation_prompt(action, "**Friends**").contains("**Friends**"));
    }
}

#[test]
fn a_confirmation_survives_the_trip_through_its_button() {
    let id = format!("{CONFIRM_BUTTON_PREFIX}opt-out:42");
    let payload = id.strip_prefix(CONFIRM_BUTTON_PREFIX).unwrap();
    assert_eq!(parse_confirmation(payload), Some((Action::OptOut, 42)));
    assert_eq!(parse_confirmation("opt-out:not-a-number"), None);
    assert_eq!(parse_confirmation("explode:42"), None);
    assert_eq!(parse_confirmation("opt-out"), None);
}

#[test]
fn choices_match_what_was_typed() {
    let names: Vec<_> = rank_guilds(guilds(), None, "OU")
        .into_iter()
        .map(|guild| guild.name)
        .collect();
    assert_eq!(names, vec!["Rouvas"]);
}

#[test]
fn the_current_server_is_offered_first_then_by_name() {
    let names: Vec<_> = rank_guilds(guilds(), Some(3), "")
        .into_iter()
        .map(|guild| guild.name)
        .collect();
    assert_eq!(names, vec!["Work", "Friends", "Rouvas"]);
}

#[test]
fn servers_someone_left_come_after_the_ones_they_are_in() {
    let names: Vec<_> = rank_guilds(vec![left(1, "Alpha"), guild(2, "Zulu")], None, "")
        .into_iter()
        .map(|guild| guild.name)
        .collect();
    assert_eq!(names, vec!["Zulu", "Alpha"]);
}

#[test]
fn choices_stop_at_what_discord_accepts() {
    let many = (0..40).map(|id| guild(id, "Server")).collect();
    assert_eq!(rank_guilds(many, None, "").len(), MAX_CHOICES);
}

#[test]
fn a_server_someone_left_is_labelled_when_the_name_has_room() {
    assert_eq!(choice_label(&left(1, "Old")), "Old (left)");
    assert_eq!(choice_label(&guild(1, "Here")), "Here");

    let longest = "w".repeat(100);
    assert!(choice_label(&left(1, &longest)).chars().count() <= MAX_CHOICE_NAME);
}

/// Preferences are per server: acting on one leaves the others alone.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn a_change_applies_only_to_the_chosen_server(pool: PgPool) -> Result<(), DbError> {
    let reply = apply(&pool, Action::OptOut, 2, 7, "**Friends**", false).await;
    assert!(reply.contains("**Friends**"), "{reply}");
    assert!(opt_outs::is_opted_out(&pool, 2, 7).await?);
    assert!(!opt_outs::is_opted_out(&pool, 1, 7).await?);

    apply(&pool, Action::Private, 2, 7, "**Friends**", false).await;
    assert!(privacy::is_private(&pool, 2, 7).await?);
    assert!(!privacy::is_private(&pool, 1, 7).await?);
    Ok(())
}

/// Whatever row is left over from before, someone who left isn't recorded.
#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn status_after_leaving_does_not_claim_they_are_recorded(
    pool: PgPool,
) -> Result<(), DbError> {
    let reply = apply(&pool, Action::Status, 2, 7, "**Old**", true).await;
    assert!(reply.contains("You've left **Old**"), "{reply}");
    assert!(!reply.contains("Your voice is recorded"), "{reply}");
    assert!(reply.contains("/recording private"), "{reply}");
    Ok(())
}

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn recorded_guilds_are_only_those_with_the_users_own_recordings(
    pool: PgPool,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO bot_instances (instance_id, role, state, heartbeat_at, started_at)
             VALUES ('test', 'active', 'active', now(), now())",
    )
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO guilds (id, owner_id, name) VALUES (1, 9, 'Mine'), (2, 9, 'Theirs')")
        .execute(&pool)
        .await?;
    let root = tempfile::tempdir().unwrap();
    let owner = RecordingOwner {
        instance_id: "test",
        recorder_id: "test-recorder",
    };
    create_fragment_in(&pool, 1, 3, 7, chrono::Utc::now(), owner, root.path()).await?;
    let owner = RecordingOwner {
        instance_id: "test",
        recorder_id: "test-recorder",
    };
    create_fragment_in(&pool, 2, 3, 8, chrono::Utc::now(), owner, root.path()).await?;

    assert_eq!(
        privacy::recorded_guilds(&pool, 7).await?,
        vec![(1, "Mine".to_string())]
    );
    Ok(())
}
