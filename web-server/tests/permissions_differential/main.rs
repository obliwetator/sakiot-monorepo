//! The permission facts as the web server loads them from its cache
//! tables, against the same random guilds loaded in memory.
//!
//! Each case generates a world (`sakiot_permissions::testing`), writes it
//! into the cache tables, and asks `GuildAccess` the same questions twice:
//! over the facts one statement loads (`load`, and `load_many` for every
//! guild at once), and over `World::facts`, which holds the world as the
//! evaluator reads it. Every answer must agree, refusals included. What the
//! evaluator makes of the facts is `sakiot-permissions`' to test (its
//! invariants and the serenity reference), on the same generator.
//!
//! `PROPTEST_CASES` overrides the number of worlds.

use std::fmt::Debug;

use proptest::prop_assert_eq;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, TestCaseError, TestRunner};
use sakiot_permissions::testing::{self, RosterState, Target, World};
use sqlx::PgPool;
use web_server::errors::AppError;
use web_server::permissions::{GuildAccess, Viewer};

/// Each world costs a few hundred loads; this keeps the test to seconds.
const DEFAULT_CASES: u32 = 48;

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn loaded_facts_answer_as_the_world_does(pool: PgPool) {
    let checked = check_worlds(testing::world(), async |world: &World| {
        store(&pool, world).await.map_err(fail)?;
        for user in 0..world.users {
            for dev in [false, true] {
                let viewer = Viewer {
                    user_id: World::user_id(user),
                    dev,
                };
                compare(&pool, world, viewer)
                    .await
                    .map_err(within(format!("{viewer:?}")))?;
            }
        }
        Ok(())
    })
    .await;
    if let Err(failure) = checked {
        panic!("{failure}");
    }
}

/// Every observed guild's access, and every role preview there, loaded
/// from the database (left) and from the world (right).
async fn compare(pool: &PgPool, world: &World, viewer: Viewer) -> Result<(), TestCaseError> {
    let guilds = observed_guilds(world);
    let ids: Vec<i64> = guilds.iter().map(|&guild| guild_id(guild)).collect();
    let mut batch = GuildAccess::load_many(pool, &ids, viewer)
        .await
        .map_err(fail)?;
    prop_assert_eq!(batch.len(), ids.len(), "one access per guild of the batch");
    for guild in guilds {
        let place = format!("guild {}", guild_id(guild));
        let expected = outcome(
            GuildAccess::new(world.facts(guild), viewer, None)
                .access()
                .cloned(),
        )?;
        let loaded = GuildAccess::load(pool, guild_id(guild), viewer, None)
            .await
            .map_err(fail)?;
        prop_assert_eq!(
            outcome(loaded.access().cloned())?,
            expected.clone(),
            "{}",
            place
        );
        let batched = batch
            .remove(&guild_id(guild))
            .ok_or_else(|| fail(format_args!("{place} missing from the batch")))?;
        prop_assert_eq!(
            outcome(batched.access().cloned())?,
            expected,
            "{} in a batch",
            place
        );
        for role in previewed_roles(world, guild) {
            let expected =
                outcome(GuildAccess::new(world.facts(guild), viewer, Some(role)).role_preview())?;
            let loaded = GuildAccess::load(pool, guild_id(guild), viewer, Some(role))
                .await
                .map_err(fail)?;
            prop_assert_eq!(
                outcome(loaded.role_preview())?,
                expected,
                "{} previewing role {}",
                place,
                role
            );
        }
    }
    Ok(())
}

/// An answer: a value, or one of the refusals `GuildAccess` gives. Anything
/// else fails the case.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome<T> {
    Ok(T),
    Forbidden,
    MembershipUnavailable,
    RoleNotFound,
}

fn outcome<T>(result: Result<T, AppError>) -> Result<Outcome<T>, TestCaseError> {
    match result {
        Ok(value) => Ok(Outcome::Ok(value)),
        Err(AppError::Forbidden) => Ok(Outcome::Forbidden),
        Err(AppError::MembershipUnavailable) => Ok(Outcome::MembershipUnavailable),
        Err(AppError::RoleNotFound) => Ok(Outcome::RoleNotFound),
        Err(error) => Err(fail(format_args!("unexpected error: {error}"))),
    }
}

/// Runs `check` on random worlds. When one fails, shrinks it with proptest's
/// value tree (simplify while the check still fails, complicate once it
/// passes) and reports the smallest failing world. proptest's own runner
/// takes a synchronous closure, and these checks need the database on
/// `#[sqlx::test]`'s current-thread runtime.
async fn check_worlds<S>(
    strategy: S,
    mut check: impl AsyncFnMut(&S::Value) -> Result<(), TestCaseError>,
) -> Result<(), String>
where
    S: Strategy,
    S::Value: Debug,
{
    let mut config = Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = DEFAULT_CASES;
    }
    let mut runner = TestRunner::new(config.clone());
    for case in 0..config.cases {
        let mut tree = strategy
            .new_tree(&mut runner)
            .map_err(|reason| format!("generating a world: {reason:?}"))?;
        let Err(error) = check(&tree.current()).await else {
            continue;
        };
        let mut smallest = (tree.current(), error);
        let mut steps = 0;
        if tree.simplify() {
            while steps < config.max_shrink_iters {
                steps += 1;
                match check(&tree.current()).await {
                    Ok(()) => {
                        if !tree.complicate() {
                            break;
                        }
                    }
                    Err(error) => {
                        smallest = (tree.current(), error);
                        if !tree.simplify() {
                            break;
                        }
                    }
                }
            }
        }
        let (world, error) = smallest;
        return Err(format!(
            "case {case} failed; smallest failing world after {steps} shrink steps:\n\
             {world:#?}\n{error}"
        ));
    }
    Ok(())
}

fn fail(error: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(error.to_string())
}

/// Prefixes a failure with where it happened.
fn within(place: String) -> impl FnOnce(TestCaseError) -> TestCaseError {
    move |error| match error {
        TestCaseError::Fail(reason) => fail(format_args!("{place}: {}", reason.message())),
        rejected @ TestCaseError::Reject(_) => rejected,
    }
}

/// Replaces the permission cache with `world`.
async fn store(pool: &PgPool, world: &World) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    // Roles, channels, overwrites and role grants cascade from `guilds`; the
    // roster cascades from its projection row.
    sqlx::raw_sql(
        "DELETE FROM guilds;
         DELETE FROM guild_projection_state;
         DELETE FROM guilds_present;
         DELETE FROM user_guilds;",
    )
    .execute(&mut *tx)
    .await?;
    for (g, guild) in world.guilds.iter().enumerate() {
        let guild_id = World::guild_id(g);
        sqlx::query("INSERT INTO guilds (id, owner_id) VALUES ($1, $2)")
            .bind(guild_id)
            .bind(World::owner_id(guild))
            .execute(&mut *tx)
            .await?;
        if guild.bot_present {
            sqlx::query("INSERT INTO guilds_present (guild_id) VALUES ($1)")
                .bind(guild_id)
                .execute(&mut *tx)
                .await?;
        }
        let everyone = guild.everyone.map(|bits| (guild_id, bits));
        let roles = guild
            .roles
            .iter()
            .enumerate()
            .map(|(r, &bits)| (World::role_id(g, r), bits));
        for (role_id, bits) in everyone.into_iter().chain(roles) {
            sqlx::query(
                "INSERT INTO roles (guild_id, role_id, permission, name)
                 VALUES ($1, $2, $3, 'role')",
            )
            .bind(guild_id)
            .bind(role_id)
            .bind(bits)
            .execute(&mut *tx)
            .await?;
        }
        for (c, channel) in guild.channels.iter().enumerate() {
            let channel_id = World::channel_id(g, c);
            sqlx::query(
                "INSERT INTO channels (channel_id, guild_id, type, name)
                 VALUES ($1, $2, $3, 'channel')",
            )
            .bind(channel_id)
            .bind(guild_id)
            .bind(channel.kind.code())
            .execute(&mut *tx)
            .await?;
            for overwrite in &channel.overwrites {
                let (target_id, kind) = match overwrite.target {
                    Target::Everyone => (guild_id, "role"),
                    Target::Role(r) => (World::role_id(g, r), "role"),
                    Target::User(u) => (World::user_id(u), "user"),
                    Target::DeletedRole => (World::deleted_role_id(g), "role"),
                };
                sqlx::query(
                    "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(channel_id)
                .bind(target_id)
                .bind(kind)
                .bind(overwrite.allow)
                .bind(overwrite.deny)
                .execute(&mut *tx)
                .await?;
            }
        }
        if guild.roster != RosterState::Missing {
            sqlx::query(
                "INSERT INTO guild_projection_state (guild_id, generation, roster_complete_at)
                 VALUES ($1, 1, CASE WHEN $2::boolean THEN now() END)",
            )
            .bind(guild_id)
            .bind(guild.roster == RosterState::Complete)
            .execute(&mut *tx)
            .await?;
        }
        for (u, member) in guild.members.iter().enumerate() {
            let user_id = World::user_id(u);
            if member.listed && guild.roster != RosterState::Missing {
                sqlx::query(
                    "INSERT INTO guild_members (guild_id, user_id, username)
                     VALUES ($1, $2, 'member')",
                )
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            }
            for (r, _) in member.roles.iter().enumerate().filter(|(_, held)| **held) {
                sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
                    .bind(user_id)
                    .bind(World::role_id(g, r))
                    .execute(&mut *tx)
                    .await?;
            }
            if let Some(owner) = member.seed {
                sqlx::query(
                    "INSERT INTO user_guilds (id, user_id, name, owner, permissions, features)
                     VALUES ($1, $2, 'guild', $3, 0, '{}')",
                )
                .bind(guild_id)
                .bind(user_id)
                .bind(owner)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await
}

/// The guilds a world is observed in: each generated one (`Some(index)`),
/// and one nothing is stored for (`None`).
fn observed_guilds(world: &World) -> Vec<Option<usize>> {
    (0..world.guilds.len()).map(Some).chain([None]).collect()
}

fn guild_id(guild: Option<usize>) -> i64 {
    World::guild_id_or_unknown(guild)
}

fn other_guild(world: &World, guild: Option<usize>) -> Option<usize> {
    (0..world.guilds.len()).find(|&other| Some(other) != guild)
}

/// `@everyone` (whose id is the guild's), every other role of the guild, a
/// deleted role, and a role of another guild.
fn previewed_roles(world: &World, guild: Option<usize>) -> Vec<i64> {
    let own = guild.into_iter().flat_map(|g| {
        (0..world.guilds[g].roles.len())
            .map(move |r| World::role_id(g, r))
            .chain([World::deleted_role_id(g)])
    });
    let foreign = other_guild(world, guild)
        .filter(|&other| !world.guilds[other].roles.is_empty())
        .map(|other| World::role_id(other, 0));
    [guild_id(guild)]
        .into_iter()
        .chain(own)
        .chain(foreign)
        .collect()
}
