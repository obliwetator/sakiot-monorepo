//! Planning a fixture guild's history from its spec and seed, and linking
//! each planned recording and clip to shared source media.

use super::*;

pub(super) struct Member {
    pub(super) id: i64,
    pub(super) insider: bool,
}

pub(super) struct Fragment {
    pub(super) session_id: i64,
    pub(super) audio_file_id: i64,
    pub(super) user_id: i64,
    pub(super) channel_id: i64,
    pub(super) start_ms: i64,
    pub(super) end_ms: i64,
    pub(super) variant: u8,
    pub(super) minutes: u32,
}

pub(super) struct Clip {
    pub(super) clip_id: String,
    pub(super) fragment: usize,
    pub(super) user_id: i64,
    pub(super) offset_s: f32,
    pub(super) created_ms: i64,
    pub(super) variant: u8,
}

/// One fixture guild's planned contents, with ids allocated, ready to insert.
pub(super) struct FixturePlan {
    pub(super) guild_id: i64,
    pub(super) members: Vec<Member>,
    pub(super) channels: Vec<i64>,
    /// Channels before this index are public; the rest admit only insiders.
    pub(super) public: usize,
    pub(super) fragments: Vec<Fragment>,
    pub(super) clips: Vec<Clip>,
    pub(super) stamps: Vec<Stamp>,
}

pub(super) struct Stamp {
    pub(super) fragment: usize,
    pub(super) stamper: i64,
    pub(super) at_ms: i64,
}

pub(super) async fn seed(
    state: &LoadtestState,
    fixture: u32,
    spec: &FixtureSpec,
) -> Result<(), AppError> {
    let guild_id = ids::guild_id(fixture);
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM guilds WHERE id = $1)")
        .bind(guild_id)
        .fetch_one(&state.pool)
        .await?;
    if exists {
        if !spec.reset {
            return Ok(());
        }
        teardown(state, fixture).await?;
    }

    let sources = media::ensure_sources().await?;
    let mut rng = fastrand::Rng::with_seed(spec.seed ^ u64::from(fixture));

    let members: Vec<Member> = (0..spec.members)
        .map(|index| Member {
            id: ids::user_id(guild_id, index),
            insider: index % 3 == 0,
        })
        .collect();
    let channels: Vec<i64> = (0..spec.voice_channels)
        .map(|index| ids::channel_id(guild_id, index))
        .collect();
    let public = (spec.voice_channels - spec.restricted_channels) as usize;

    // Plan the history before allocating ids, so each table is one insert.
    let now = Utc::now();
    let today = Utc
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .ok_or(AppError::InternalError)?;
    let mut fragments = Vec::new();
    let mut sittings: Vec<std::ops::Range<usize>> = Vec::new();
    let evening_ms = 12 * 3_600_000 / i64::from(spec.sittings_per_day);
    for day in 1..=i64::from(spec.history_days) {
        for sitting in 0..i64::from(spec.sittings_per_day) {
            let start = today - Duration::days(day)
                + Duration::hours(8)
                + Duration::milliseconds(sitting * evening_ms + rng.i64(0..1_800_000));
            let restricted = public < channels.len() && rng.u8(0..100) < 15;
            let channel_id = if restricted {
                channels[rng.usize(public..channels.len())]
            } else {
                channels[rng.usize(0..public)]
            };
            let pool: Vec<i64> = members
                .iter()
                .filter(|member| !restricted || member.insider)
                .map(|member| member.id)
                .collect();
            let wanted = rng.u32(spec.participants_min..=spec.participants_max) as usize;
            let mut chosen = pool;
            rng.shuffle(&mut chosen);
            chosen.truncate(wanted.max(1));

            let first = fragments.len();
            for user_id in chosen {
                let minutes = pick_minutes(&mut rng);
                let variant = rng.u8(0..VARIANTS);
                let source = sources
                    .recording(variant, minutes)
                    .ok_or(AppError::InternalError)?;
                let start_ms = start.timestamp_millis() + rng.i64(0..180_000);
                fragments.push(Fragment {
                    session_id: 0,
                    audio_file_id: 0,
                    user_id,
                    channel_id,
                    start_ms,
                    end_ms: start_ms + source.duration_ms,
                    variant,
                    minutes,
                });
            }
            sittings.push(first..fragments.len());
        }
    }

    let mut clips = Vec::new();
    let mut stamps = Vec::new();
    for range in &sittings {
        let in_sitting: Vec<usize> = range.clone().collect();
        if in_sitting.is_empty() {
            continue;
        }
        for _ in 0..rng.u32(0..=spec.clips_per_sitting) {
            let fragment = in_sitting[rng.usize(0..in_sitting.len())];
            let clipper = fragments[in_sitting[rng.usize(0..in_sitting.len())]].user_id;
            let span_s =
                ((fragments[fragment].end_ms - fragments[fragment].start_ms) / 1000).max(30);
            let offset_s = rng.i64(0..span_s - 20) as f32;
            clips.push(Clip {
                clip_id: uuid::Uuid::new_v4().to_string(),
                fragment,
                user_id: clipper,
                offset_s,
                created_ms: fragments[fragment].start_ms + (offset_s as i64 + 30) * 1000,
                variant: rng.u8(0..VARIANTS),
            });
        }
        for _ in 0..rng.u32(0..=spec.stamps_per_sitting) {
            let fragment = in_sitting[rng.usize(0..in_sitting.len())];
            let stamper = fragments[in_sitting[rng.usize(0..in_sitting.len())]].user_id;
            let f = &fragments[fragment];
            stamps.push(Stamp {
                fragment,
                stamper,
                at_ms: rng.i64(f.start_ms..f.end_ms),
            });
        }
    }

    let session_ids = allocate(&state.pool, "recording_sessions_id_seq", fragments.len()).await?;
    let audio_ids = allocate(&state.pool, "audio_files_id_seq", fragments.len()).await?;
    for (index, fragment) in fragments.iter_mut().enumerate() {
        fragment.session_id = session_ids[index];
        fragment.audio_file_id = audio_ids[index];
    }

    link_media(&fragments, &clips, &sources, guild_id).await?;
    let plan = FixturePlan {
        guild_id,
        members,
        channels,
        public,
        fragments,
        clips,
        stamps,
    };
    insert_all(state, spec, &sources, &plan).await?;
    tracing::info!(
        fixture,
        guild_id,
        sessions = plan.fragments.len(),
        clips = plan.clips.len(),
        stamps = plan.stamps.len(),
        "load-test fixture seeded"
    );
    Ok(())
}

/// Recording lengths weighted towards an hour or so, as evening calls run.
fn pick_minutes(rng: &mut fastrand::Rng) -> u32 {
    const WEIGHTS: [u32; 6] = [1, 2, 3, 4, 3, 2];
    let total: u32 = WEIGHTS.iter().sum();
    let mut roll = rng.u32(0..total);
    for (minutes, weight) in DURATIONS_MINUTES.iter().zip(WEIGHTS) {
        if roll < weight {
            return *minutes;
        }
        roll -= weight;
    }
    DURATIONS_MINUTES[0]
}

async fn allocate(
    pool: &Pool<Postgres>,
    sequence: &str,
    count: usize,
) -> Result<Vec<i64>, AppError> {
    Ok(
        sqlx::query_scalar("SELECT nextval($1::regclass) FROM generate_series(1, $2)")
            .bind(sequence)
            .bind(count as i64)
            .fetch_all(pool)
            .await?,
    )
}

fn utc(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Utc::now)
}

pub(super) fn recording_key(guild_id: i64, fragment: &Fragment) -> sakiot_paths::RecordingKey {
    let start = utc(fragment.start_ms);
    sakiot_paths::RecordingKey::new(
        guild_id,
        fragment.channel_id,
        start.year(),
        start.month(),
        sakiot_paths::RecordingKey::stem_for(fragment.start_ms, fragment.user_id),
    )
}

pub(super) fn clip_saved_name(clip: &Clip) -> String {
    let created = utc(clip.created_ms);
    format!(
        "{:04}/{:02}/{}.ogg",
        created.year(),
        created.month(),
        clip.clip_id
    )
}

async fn link_media(
    fragments: &[Fragment],
    clips: &[Clip],
    sources: &Sources,
    guild_id: i64,
) -> Result<(), AppError> {
    let roots = sakiot_paths::DataRoots::from_env();
    let mut links = Vec::with_capacity(fragments.len() + clips.len());
    for fragment in fragments {
        let source = sources
            .recording(fragment.variant, fragment.minutes)
            .ok_or(AppError::InternalError)?;
        let dest = recording_key(guild_id, fragment).recording_path(&roots.recordings_str());
        links.push((source.path.clone(), dest));
    }
    for clip in clips {
        let source = sources.clip(clip.variant).ok_or(AppError::InternalError)?;
        links.push((source.path.clone(), roots.clips.join(clip_saved_name(clip))));
    }
    tokio::task::spawn_blocking(move || {
        links
            .iter()
            .try_for_each(|(source, dest)| media::link(source, dest))
    })
    .await
    .map_err(|_| AppError::InternalError)??;
    Ok(())
}
