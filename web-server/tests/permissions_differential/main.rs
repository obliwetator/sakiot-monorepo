//! Random guilds through every public check in `web_server::permissions`,
//! against the implementation they replaced (`permissions::legacy`).
//!
//! Each case generates a world (`sakiot_permissions::testing`), writes it
//! into the permission cache tables, and records what every public check
//! answers for every viewer, probed channel and role preview, once through
//! the evaluator and once through the old checks. The two must agree
//! exactly, refusals included. The test also pins the invariants the checks
//! keep and a small model of membership and guild-level permissions, and
//! fails if the generator stops reaching an interesting outcome, so the
//! comparison cannot pass vacuously.
//!
//! `PROPTEST_CASES` overrides the number of worlds.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Debug;

use actix_web::test::TestRequest;
use actix_web::{HttpMessage, HttpRequest, web};
use futures_util::future::try_join_all;
use jsonwebtoken::{DecodingKey, EncodingKey};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, TestCaseError, TestRunner};
use proptest::{prop_assert, prop_assert_eq};
use sakiot_permissions::testing::{self, RosterState, Target, World};
use sqlx::PgPool;
use web_server::auth::{Access, AccessKeys, AuthKind, Token};
use web_server::errors::AppError;
use web_server::permissions::{Permissions, SubscriptionAccess, Viewer, begin_snapshot};

const ADMINISTRATOR: i64 = Permissions::ADMINISTRATOR.bits();
const MANAGE_GUILD: i64 = Permissions::MANAGE_GUILD.bits();

/// Each world costs a few hundred checks; this keeps the test to seconds.
const DEFAULT_CASES: u32 = 48;

#[sqlx::test(migrations = "../sakiot-db/migrations")]
async fn permission_checks_match_the_old_ones_and_keep_their_invariants(pool: PgPool) {
    let pool = web::Data::new(pool);
    let mut coverage = Coverage::default();
    let checked = check_worlds(testing::world(), async |world: &World| {
        store(&pool, world).await.map_err(fail)?;
        for guild in observed_guilds(world) {
            let place = format!("guild {}", guild_id(guild));
            let observed = current::observe(&pool, world, guild).await?;
            let reference = legacy::observe(&pool, world, guild).await?;
            compare(&observed, &reference).map_err(within(place.clone()))?;
            check(world, guild, &observed, &mut coverage).map_err(within(place))?;
        }
        Ok(())
    })
    .await;
    if let Err(failure) = checked.and_then(|()| coverage.require_every_outcome()) {
        panic!("{failure}");
    }
}

/// The evaluator's answers must be the old checks' answers.
fn compare(observed: &GuildObservation, reference: &GuildObservation) -> Result<(), TestCaseError> {
    prop_assert_eq!(&observed.role_bits, &reference.role_bits, "bits of roles");
    prop_assert_eq!(observed.viewers.len(), reference.viewers.len());
    for (new, old) in observed.viewers.iter().zip(&reference.viewers) {
        prop_assert_eq!(
            new,
            old,
            "the evaluator (left) against the old checks (right)"
        );
    }
    Ok(())
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

/// Every channel of the guild, of every type, and one of another guild.
fn probed_channels(world: &World, guild: Option<usize>) -> Vec<i64> {
    let own = guild
        .into_iter()
        .flat_map(|g| (0..world.guilds[g].channels.len()).map(move |c| World::channel_id(g, c)));
    let foreign = other_guild(world, guild)
        .filter(|&other| !world.guilds[other].channels.is_empty())
        .map(|other| World::channel_id(other, 0));
    own.chain(foreign).collect()
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

/// A check's answer: a value, or one of the refusals the checks give.
/// Anything else (a database error) fails the case.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome<T> {
    Ok(T),
    Forbidden,
    MembershipUnavailable,
    RoleNotFound,
}

fn outcome<T, U>(
    result: Result<T, AppError>,
    map: impl FnOnce(T) -> U,
) -> Result<Outcome<U>, TestCaseError> {
    match result {
        Ok(value) => Ok(Outcome::Ok(map(value))),
        Err(AppError::Forbidden) => Ok(Outcome::Forbidden),
        Err(AppError::MembershipUnavailable) => Ok(Outcome::MembershipUnavailable),
        Err(AppError::RoleNotFound) => Ok(Outcome::RoleNotFound),
        Err(error) => Err(fail(format_args!("unexpected error: {error}"))),
    }
}

fn sorted(channels: HashSet<i64>) -> BTreeSet<i64> {
    channels.into_iter().collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Subscription {
    tree: BTreeSet<i64>,
    media: BTreeSet<i64>,
    presence: BTreeSet<i64>,
    manager: bool,
}

impl From<SubscriptionAccess> for Subscription {
    fn from(access: SubscriptionAccess) -> Self {
        Self {
            tree: sorted(access.tree_channels),
            media: sorted(access.media_channels),
            presence: sorted(access.presence_channels),
            manager: access.manager,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GuildObservation {
    /// `get_combined_perm_for_role`, which takes no viewer.
    role_bits: BTreeMap<i64, Outcome<i64>>,
    viewers: Vec<ViewerObservation>,
}

/// One viewer's answers in one guild.
#[derive(Debug, PartialEq, Eq)]
struct ViewerObservation {
    /// An index into the world's users.
    user: usize,
    viewer: Viewer,
    combined: Outcome<i64>,
    member: Outcome<()>,
    /// `require_guild_manager`, which answers with the viewer's id.
    manager: Outcome<i64>,
    visible: Outcome<BTreeSet<i64>>,
    listing: Outcome<BTreeSet<i64>>,
    presence: Outcome<BTreeSet<i64>>,
    subscription: Outcome<Subscription>,
    /// `require_channel_access` per probed channel.
    channel_access: BTreeMap<i64, Outcome<()>>,
    previews: BTreeMap<i64, PreviewObservation>,
}

/// One viewer's answers when previewing one role.
#[derive(Debug, PartialEq, Eq)]
struct PreviewObservation {
    /// `require_role_preview`.
    gate: Outcome<()>,
    /// `role_access_for_preview`: (viewable, joinable) per channel.
    access: Outcome<BTreeMap<i64, (bool, bool)>>,
    listing: Outcome<BTreeSet<i64>>,
    presence: Outcome<BTreeSet<i64>>,
    subscription: Outcome<Subscription>,
}

/// The observation of one implementation of the checks: `current` is the
/// evaluator, `legacy` the checks it replaced.
macro_rules! observer {
    ($name:ident, $($checks:ident)::+) => {
        mod $name {
            use super::*;
            use $($checks)::+ as checks;

            pub(super) async fn observe(
                pool: &web::Data<PgPool>,
                world: &World,
                guild: Option<usize>,
            ) -> Result<GuildObservation, TestCaseError> {
                let guild_id = guild_id(guild);
                let channels = probed_channels(world, guild);
                let roles = previewed_roles(world, guild);
                let mut role_bits = BTreeMap::new();
                for &role in &roles {
                    let bits = checks::get_combined_perm_for_role(pool, guild_id, role).await;
                    role_bits.insert(role, outcome(bits, |bits| bits.bits())?);
                }
                let viewers = (0..world.users).flat_map(|user| {
                    [false, true].map(|dev| {
                        let viewer = Viewer {
                            user_id: World::user_id(user),
                            dev,
                        };
                        observe_viewer(pool, guild_id, user, viewer, &channels, &roles)
                    })
                });
                Ok(GuildObservation {
                    role_bits,
                    viewers: try_join_all(viewers).await?,
                })
            }

            async fn observe_viewer(
                pool: &web::Data<PgPool>,
                guild_id: i64,
                user: usize,
                viewer: Viewer,
                channels: &[i64],
                roles: &[i64],
            ) -> Result<ViewerObservation, TestCaseError> {
                let request = request_for(viewer).map_err(fail)?;
                let mut channel_access = BTreeMap::new();
                for &channel in channels {
                    let access =
                        checks::require_channel_access(pool, guild_id, channel, viewer).await;
                    channel_access.insert(channel, outcome(access, |()| ())?);
                }
                let mut previews = BTreeMap::new();
                for &role in roles {
                    let preview = PreviewObservation {
                        gate: outcome(
                            checks::require_role_preview(&request, pool, guild_id, Some(role))
                                .await,
                            |()| (),
                        )?,
                        access: outcome(
                            checks::role_access_for_preview(pool, guild_id, viewer, role).await,
                            |access| {
                                access
                                    .into_values()
                                    .map(|channel| {
                                        let access = (channel.viewable, channel.joinable);
                                        (channel.channel_id, access)
                                    })
                                    .collect()
                            },
                        )?,
                        listing: outcome(
                            checks::listing_channels_for(pool, guild_id, viewer, Some(role)).await,
                            sorted,
                        )?,
                        presence: presence(pool, guild_id, viewer, Some(role)).await?,
                        subscription: outcome(
                            checks::subscription_access(pool, guild_id, viewer, Some(role)).await,
                            Subscription::from,
                        )?,
                    };
                    previews.insert(role, preview);
                }
                Ok(ViewerObservation {
                    user,
                    viewer,
                    combined: outcome(
                        checks::get_combined_perm_for_user(pool, guild_id, viewer).await,
                        |bits| bits.bits(),
                    )?,
                    member: outcome(
                        checks::require_guild_member(pool, guild_id, viewer).await,
                        |()| (),
                    )?,
                    manager: outcome(
                        checks::require_guild_manager(&request, pool, guild_id).await,
                        |user_id| user_id,
                    )?,
                    visible: outcome(
                        checks::visible_channels_for_user(pool, guild_id, viewer).await,
                        sorted,
                    )?,
                    listing: outcome(
                        checks::listing_channels_for(pool, guild_id, viewer, None).await,
                        sorted,
                    )?,
                    presence: presence(pool, guild_id, viewer, None).await?,
                    subscription: outcome(
                        checks::subscription_access(pool, guild_id, viewer, None).await,
                        Subscription::from,
                    )?,
                    channel_access,
                    previews,
                })
            }

            /// `presence_channels` reads on the caller's connection, as the presence
            /// endpoint and the realtime hub call it.
            async fn presence(
                pool: &PgPool,
                guild_id: i64,
                viewer: Viewer,
                as_role: Option<i64>,
            ) -> Result<Outcome<BTreeSet<i64>>, TestCaseError> {
                let mut snapshot = begin_snapshot(pool).await.map_err(fail)?;
                let channels =
                    checks::presence_channels(&mut snapshot, guild_id, viewer, as_role).await;
                snapshot.commit().await.map_err(fail)?;
                outcome(channels, sorted)
            }
        }
    };
}

observer!(current, web_server::permissions);
observer!(legacy, web_server::permissions::legacy);

/// A request carrying the viewer's access token, as the auth middleware
/// leaves it, for the checks that read the viewer from the request.
fn request_for(viewer: Viewer) -> Result<HttpRequest, AppError> {
    let keys = AccessKeys {
        access_encode: EncodingKey::from_secret(b"harness"),
        refresh_encode: EncodingKey::from_secret(b"harness"),
        access_decode: DecodingKey::from_secret(b"harness"),
        refresh_decode: DecodingKey::from_secret(b"harness"),
    };
    let kind = if viewer.dev {
        AuthKind::Dev
    } else {
        AuthKind::Discord
    };
    let token = Token::<Access>::encode(
        viewer.user_id,
        kind,
        "harness".into(),
        "csrf".into(),
        &keys.access_encode,
    )?;
    let request = TestRequest::default().to_http_request();
    request
        .extensions_mut()
        .insert(Token::<Access>::decode(&token, &keys)?);
    Ok(request)
}

/// Membership as the checks document it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Membership {
    Owner,
    Member,
    Outsider,
    Unavailable,
}

fn membership(world: &World, guild: Option<usize>, user: usize, viewer: Viewer) -> Membership {
    let Some(guild) = guild.map(|g| &world.guilds[g]) else {
        return Membership::Outsider;
    };
    if World::owner_id(guild) == viewer.user_id {
        return Membership::Owner;
    }
    let member = &guild.members[user];
    if viewer.dev {
        return match member.seed {
            Some(true) => Membership::Owner,
            Some(false) => Membership::Member,
            None => Membership::Outsider,
        };
    }
    match guild.roster {
        RosterState::Complete if member.listed => Membership::Member,
        RosterState::Complete => Membership::Outsider,
        RosterState::Incomplete | RosterState::Missing if guild.bot_present => {
            Membership::Unavailable
        }
        RosterState::Incomplete | RosterState::Missing => Membership::Outsider,
    }
}

/// A member's guild-level bits: `@everyone` and the roles they hold here.
fn member_bits(world: &World, guild: usize, user: usize) -> i64 {
    let guild = &world.guilds[guild];
    guild
        .roles
        .iter()
        .zip(&guild.members[user].roles)
        .filter(|(_, held)| **held)
        .fold(guild.everyone.unwrap_or(0), |bits, (role, _)| bits | role)
}

/// The bits of `role` if it is a stored role of the guild (`@everyone`
/// included); `None` for a deleted or foreign role.
fn stored_role(world: &World, guild: Option<usize>, role: i64) -> Option<i64> {
    let g = guild?;
    let spec = &world.guilds[g];
    if role == World::guild_id(g) {
        return spec.everyone;
    }
    (0..spec.roles.len())
        .find(|&r| World::role_id(g, r) == role)
        .map(|r| spec.roles[r])
}

fn recorded_channels(world: &World, guild: Option<usize>) -> BTreeSet<i64> {
    guild
        .into_iter()
        .flat_map(|g| {
            world.guilds[g]
                .channels
                .iter()
                .enumerate()
                .filter(|(_, channel)| channel.kind.recorded())
                .map(move |(c, _)| World::channel_id(g, c))
        })
        .collect()
}

fn check(
    world: &World,
    guild: Option<usize>,
    observed: &GuildObservation,
    coverage: &mut Coverage,
) -> Result<(), TestCaseError> {
    let everyone = guild.and_then(|g| world.guilds[g].everyone).unwrap_or(0);
    for (&role, bits) in &observed.role_bits {
        let own = stored_role(world, guild, role).unwrap_or(0);
        prop_assert_eq!(bits, &Outcome::Ok(everyone | own), "bits of role {}", role);
    }
    for viewer in &observed.viewers {
        check_viewer(world, guild, viewer, coverage)
            .map_err(within(format!("{:?}", viewer.viewer)))?;
    }
    Ok(())
}

fn check_viewer(
    world: &World,
    guild: Option<usize>,
    observed: &ViewerObservation,
    coverage: &mut Coverage,
) -> Result<(), TestCaseError> {
    let viewer = observed.viewer;
    let everyone = guild.and_then(|g| world.guilds[g].everyone).unwrap_or(0);
    let membership = membership(world, guild, observed.user, viewer);
    coverage.membership(world, guild, observed.user, viewer, membership);
    let bits = match (membership, guild) {
        (Membership::Owner, _) => Permissions::all().bits(),
        (Membership::Member, Some(g)) => member_bits(world, g, observed.user),
        _ => return check_refused(world, guild, observed, membership),
    };
    prop_assert_eq!(&observed.combined, &Outcome::Ok(bits));
    prop_assert_eq!(&observed.member, &Outcome::Ok(()));
    let manager = bits & (ADMINISTRATOR | MANAGE_GUILD) != 0;
    let expected_manager = if manager {
        Outcome::Ok(viewer.user_id)
    } else {
        Outcome::Forbidden
    };
    prop_assert_eq!(&observed.manager, &expected_manager);

    let (Outcome::Ok(visible), Outcome::Ok(presence)) = (&observed.visible, &observed.presence)
    else {
        return Err(fail(format_args!(
            "a member was refused channels: {:?}, presence {:?}",
            observed.visible, observed.presence
        )));
    };
    let recorded = recorded_channels(world, guild);
    prop_assert!(
        visible.is_subset(presence),
        "a joinable channel is viewable"
    );
    prop_assert!(
        presence.is_subset(&recorded),
        "only voice and stage channels"
    );
    if bits & ADMINISTRATOR != 0 {
        prop_assert_eq!(visible, &recorded, "ADMINISTRATOR bypasses overwrites");
        prop_assert_eq!(presence, &recorded, "ADMINISTRATOR bypasses overwrites");
    }
    prop_assert_eq!(&observed.listing, &observed.visible);
    let subscription = Subscription {
        tree: visible.clone(),
        media: visible.clone(),
        presence: presence.clone(),
        manager,
    };
    prop_assert_eq!(&observed.subscription, &Outcome::Ok(subscription));
    for (channel, access) in &observed.channel_access {
        let expected = if visible.contains(channel) {
            Outcome::Ok(())
        } else {
            Outcome::Forbidden
        };
        prop_assert_eq!(access, &expected, "channel {}", channel);
    }
    if membership == Membership::Member && bits & ADMINISTRATOR == 0 {
        coverage.member_channels(visible, presence, &recorded);
    }

    for (&role, preview) in &observed.previews {
        let gate = if manager {
            Outcome::Ok(())
        } else {
            Outcome::Forbidden
        };
        prop_assert_eq!(&preview.gate, &gate, "role {}", role);
        let Some(role_bits) = stored_role(world, guild, role) else {
            prop_assert_eq!(&preview.access, &Outcome::RoleNotFound, "role {}", role);
            prop_assert_eq!(&preview.listing, &Outcome::RoleNotFound, "role {}", role);
            prop_assert_eq!(&preview.presence, &Outcome::RoleNotFound, "role {}", role);
            let subscription = if manager {
                Outcome::RoleNotFound
            } else {
                Outcome::Forbidden
            };
            prop_assert_eq!(&preview.subscription, &subscription, "role {}", role);
            if manager {
                coverage.reach(Reached::PreviewOfMissingRole);
            }
            continue;
        };
        let (Outcome::Ok(access), Outcome::Ok(role_presence)) =
            (&preview.access, &preview.presence)
        else {
            return Err(fail(format_args!(
                "role {role} preview refused: {:?}, presence {:?}",
                preview.access, preview.presence
            )));
        };
        let channels: BTreeSet<i64> = access.keys().copied().collect();
        prop_assert_eq!(
            &channels,
            visible,
            "a preview lists the previewer's channels"
        );
        prop_assert!(
            access
                .values()
                .all(|&(viewable, joinable)| viewable || !joinable),
            "role {} preview joins a channel it cannot view",
            role
        );
        let joinable: BTreeSet<i64> = access
            .iter()
            .filter(|(_, (_, joinable))| *joinable)
            .map(|(&channel, _)| channel)
            .collect();
        prop_assert_eq!(
            &preview.listing,
            &Outcome::Ok(joinable.clone()),
            "role {}",
            role
        );
        prop_assert!(
            role_presence.is_subset(presence),
            "role {} preview shows presence the previewer cannot see",
            role
        );
        if (everyone | role_bits) & ADMINISTRATOR != 0 {
            prop_assert_eq!(&joinable, visible, "role {}", role);
            prop_assert_eq!(role_presence, presence, "role {}", role);
        }
        let subscription = if manager {
            Outcome::Ok(Subscription {
                tree: channels,
                media: joinable.clone(),
                presence: role_presence.clone(),
                manager: true,
            })
        } else {
            Outcome::Forbidden
        };
        prop_assert_eq!(&preview.subscription, &subscription, "role {}", role);
        if manager && joinable.len() < visible.len() {
            coverage.reach(Reached::PreviewJoinsLess);
        }
    }
    Ok(())
}

/// Outsiders and viewers whose membership is unknown: every check refuses,
/// except that an outsider's guild-level bits are empty and a preview of a
/// role outside the guild is `RoleNotFound` before membership is checked.
fn check_refused(
    world: &World,
    guild: Option<usize>,
    observed: &ViewerObservation,
    membership: Membership,
) -> Result<(), TestCaseError> {
    fn refused<T>(membership: Membership) -> Outcome<T> {
        if membership == Membership::Unavailable {
            Outcome::MembershipUnavailable
        } else {
            Outcome::Forbidden
        }
    }
    fn refused_preview<T>(membership: Membership, stored: bool) -> Outcome<T> {
        if stored {
            refused(membership)
        } else {
            Outcome::RoleNotFound
        }
    }
    let combined = if membership == Membership::Unavailable {
        Outcome::MembershipUnavailable
    } else {
        Outcome::Ok(0)
    };
    prop_assert_eq!(&observed.combined, &combined);
    prop_assert_eq!(&observed.member, &refused(membership));
    prop_assert_eq!(&observed.manager, &refused(membership));
    prop_assert_eq!(&observed.visible, &refused(membership));
    prop_assert_eq!(&observed.listing, &refused(membership));
    prop_assert_eq!(&observed.presence, &refused(membership));
    prop_assert_eq!(&observed.subscription, &refused(membership));
    for (channel, access) in &observed.channel_access {
        prop_assert_eq!(access, &refused(membership), "channel {}", channel);
    }
    for (&role, preview) in &observed.previews {
        let stored = stored_role(world, guild, role).is_some();
        prop_assert_eq!(&preview.gate, &refused(membership), "role {}", role);
        prop_assert_eq!(
            &preview.access,
            &refused_preview(membership, stored),
            "role {}",
            role
        );
        prop_assert_eq!(
            &preview.listing,
            &refused_preview(membership, stored),
            "role {}",
            role
        );
        prop_assert_eq!(&preview.presence, &refused(membership), "role {}", role);
        prop_assert_eq!(&preview.subscription, &refused(membership), "role {}", role);
    }
    Ok(())
}

/// Outcomes the generator must reach, so a comparison over these worlds
/// cannot pass because every world is trivial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Reached {
    Owner,
    Member,
    Outsider,
    MembershipUnavailable,
    DevOwnerBySeed,
    DevMember,
    /// A member without ADMINISTRATOR who joins none of the guild's voice
    /// channels, some of them, or views one without joining it.
    MemberJoinsNone,
    MemberJoinsSome,
    MemberViewsWithoutJoining,
    ManagerWhoIsNotOwner,
    /// A manager's preview of a role that joins fewer channels than they do.
    PreviewJoinsLess,
    /// A manager's preview of a deleted or foreign role.
    PreviewOfMissingRole,
}

const EVERY_OUTCOME: [Reached; 12] = [
    Reached::Owner,
    Reached::Member,
    Reached::Outsider,
    Reached::MembershipUnavailable,
    Reached::DevOwnerBySeed,
    Reached::DevMember,
    Reached::MemberJoinsNone,
    Reached::MemberJoinsSome,
    Reached::MemberViewsWithoutJoining,
    Reached::ManagerWhoIsNotOwner,
    Reached::PreviewJoinsLess,
    Reached::PreviewOfMissingRole,
];

#[derive(Debug, Default)]
struct Coverage(BTreeSet<Reached>);

impl Coverage {
    fn reach(&mut self, outcome: Reached) {
        self.0.insert(outcome);
    }

    fn membership(
        &mut self,
        world: &World,
        guild: Option<usize>,
        user: usize,
        viewer: Viewer,
        membership: Membership,
    ) {
        let seeded = guild.and_then(|g| world.guilds[g].members[user].seed);
        let reached = match (membership, viewer.dev, seeded) {
            (Membership::Owner, true, Some(true)) => Reached::DevOwnerBySeed,
            (Membership::Member, true, _) => Reached::DevMember,
            (_, true, _) => return,
            (Membership::Owner, false, _) => Reached::Owner,
            (Membership::Member, false, _) => Reached::Member,
            (Membership::Outsider, false, _) => Reached::Outsider,
            (Membership::Unavailable, false, _) => Reached::MembershipUnavailable,
        };
        self.reach(reached);
        if membership == Membership::Member
            && guild
                .is_some_and(|g| member_bits(world, g, user) & (ADMINISTRATOR | MANAGE_GUILD) != 0)
        {
            self.reach(Reached::ManagerWhoIsNotOwner);
        }
    }

    fn member_channels(
        &mut self,
        visible: &BTreeSet<i64>,
        presence: &BTreeSet<i64>,
        recorded: &BTreeSet<i64>,
    ) {
        if recorded.is_empty() {
            return;
        }
        if visible.is_empty() {
            self.reach(Reached::MemberJoinsNone);
        } else if visible.len() < recorded.len() {
            self.reach(Reached::MemberJoinsSome);
        }
        if presence.len() > visible.len() {
            self.reach(Reached::MemberViewsWithoutJoining);
        }
    }

    fn require_every_outcome(&self) -> Result<(), String> {
        let missing: Vec<Reached> = EVERY_OUTCOME
            .into_iter()
            .filter(|outcome| !self.0.contains(outcome))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!("the generator never reached {missing:?}"))
        }
    }
}
