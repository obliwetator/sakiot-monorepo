use super::*;

fn interval(start_ms: i64, end_ms: i64) -> MixInterval {
    MixInterval { start_ms, end_ms }
}

fn session_access(state: &str) -> SessionAccess {
    SessionAccess {
        session_id: 1,
        guild_id: 2,
        user_id: 3,
        starting_channel_id: 4,
        state: state.into(),
        started_at_ms: 1_000,
        ended_at_ms: Some(9_000),
        pause_started_at_ms: Some(5_000),
    }
}

fn settings_plan() -> MixPlan {
    let tracks = vec![
        ChannelMixTrack {
            user_id: "3".into(),
            display_name: Some("Anchor".into()),
            is_anchor: true,
            segments: Vec::new(),
        },
        ChannelMixTrack {
            user_id: "7".into(),
            display_name: Some("Contributor".into()),
            is_anchor: false,
            segments: Vec::new(),
        },
    ];
    let settings = default_generation_settings(&tracks);
    MixPlan {
        session_id: 1,
        scope: ChannelMixScope::SelectedSession,
        duration_ms: 1_000,
        contributors: Vec::new(),
        sources: Vec::new(),
        participants: Vec::new(),
        tracks,
        cache_dir: PathBuf::from("cache"),
        source_fingerprint: "source".into(),
        fingerprint: mix_fingerprint("source", &settings),
        settings,
    }
}

#[test]
fn generation_settings_default_missing_participants_and_normalize_ids() {
    let plan = settings_plan();
    let settings = canonical_generation_settings(
        &plan,
        vec![ChannelMixParticipantSettings {
            user_id: "07".into(),
            gain_db: 3.5,
            muted: true,
        }],
    )
    .expect("valid settings");
    assert_eq!(
        settings.participants,
        vec![
            ChannelMixParticipantSettings {
                user_id: "3".into(),
                gain_db: 0.0,
                muted: false,
            },
            ChannelMixParticipantSettings {
                user_id: "7".into(),
                gain_db: 3.5,
                muted: true,
            },
        ]
    );
}

#[test]
fn generation_settings_reject_duplicate_unknown_invalid_and_all_muted() {
    let plan = settings_plan();
    let duplicate = canonical_generation_settings(
        &plan,
        vec![
            ChannelMixParticipantSettings {
                user_id: "3".into(),
                gain_db: 0.0,
                muted: false,
            },
            ChannelMixParticipantSettings {
                user_id: "03".into(),
                gain_db: 0.0,
                muted: false,
            },
        ],
    );
    assert!(
        matches!(duplicate, Err(AppError::BadRequest(message)) if message.contains("Duplicate"))
    );

    let unknown = canonical_generation_settings(
        &plan,
        vec![ChannelMixParticipantSettings {
            user_id: "99".into(),
            gain_db: 0.0,
            muted: false,
        }],
    );
    assert!(matches!(unknown, Err(AppError::BadRequest(message)) if message.contains("Unknown")));

    let invalid = canonical_generation_settings(
        &plan,
        vec![ChannelMixParticipantSettings {
            user_id: "3".into(),
            gain_db: 12.1,
            muted: false,
        }],
    );
    assert!(matches!(invalid, Err(AppError::BadRequest(message)) if message.contains("between")));

    let all_muted = canonical_generation_settings(
        &plan,
        vec![
            ChannelMixParticipantSettings {
                user_id: "3".into(),
                gain_db: 0.0,
                muted: true,
            },
            ChannelMixParticipantSettings {
                user_id: "7".into(),
                gain_db: 0.0,
                muted: true,
            },
        ],
    );
    assert!(matches!(all_muted, Err(AppError::BadRequest(message)) if message.contains("unmuted")));
}

#[test]
fn tracks_include_anchor_and_aligned_authenticated_source_metadata() {
    let anchor = AudioFragment {
        id: 11,
        guild_id: 1,
        channel_id: 10,
        user_id: 3,
        recording_session_id: Some(1),
        file_name: "1000-3".into(),
        year: 2026,
        month: 8,
        start_ms: 1_500,
        end_ms: Some(3_000),
        segment_index: Some(0),
        live: false,
    };
    let contributor = AudioFragment {
        id: 22,
        user_id: 7,
        recording_session_id: Some(2),
        file_name: "1200-7".into(),
        start_ms: 1_000,
        end_ms: Some(3_000),
        live: true,
        ..anchor.clone()
    };
    let participants = vec![
        ChannelMixParticipant {
            user_id: "3".into(),
            display_name: Some("Anchor".into()),
            session_ids: vec!["1".into()],
            source_count: 1,
        },
        ChannelMixParticipant {
            user_id: "7".into(),
            display_name: Some("Contributor".into()),
            session_ids: vec!["2".into()],
            source_count: 1,
        },
    ];
    let mut sources = Vec::new();
    add_fragment_sources(
        &mut sources,
        std::slice::from_ref(&anchor),
        std::slice::from_ref(&contributor),
        7,
        1_000,
        3_000,
    );
    let tracks = build_tracks(&participants, &sources, Some(3), 1_000);
    assert_eq!(tracks.len(), 2);
    assert!(tracks[0].is_anchor);
    let segment = &tracks[1].segments[0];
    assert_eq!(segment.start_ms, 500);
    assert_eq!(segment.end_ms, 2_000);
    assert_eq!(segment.source_offset_ms, 500);
    assert!(segment.live);
    assert_eq!(segment.id, "22:1500");
    assert_eq!(segment.recording_session_id.as_deref(), Some("2"));
    assert_eq!(segment.media_url, "/api/audio/sessions/2/segments/22");
    assert_eq!(
        segment.hls_playlist_url,
        "/api/audio/sessions/2/live/22/playlist.m3u8"
    );
    assert_eq!(
        segment.waveform_url,
        "/api/audio/waveform/1/10/2026/08/1200-7"
    );
    let mut extended_source = sources[0].clone();
    extended_source.overlap_end_ms += 1_000;
    assert_eq!(
        source_segment(&sources[0], 1_000).id,
        source_segment(&extended_source, 1_000).id
    );
}

#[test]
fn anchor_wait_reason_distinguishes_active_and_pending_sessions() {
    assert!(anchor_wait_reason(&session_access("finalized")).is_none());
    assert_eq!(
        anchor_wait_reason(&session_access("active"))
            .expect("active reason")
            .code,
        "active_anchor"
    );
    assert_eq!(
        anchor_wait_reason(&session_access("pending"))
            .expect("pending reason")
            .message,
        "The anchor recording is paused and awaiting finalization."
    );
}

#[test]
fn staggered_intervals_are_trimmed_to_the_shared_half_open_range() {
    assert_eq!(
        intersect_mix_intervals(interval(1_000, 4_000), interval(2_500, 5_000)),
        Some(interval(2_500, 4_000))
    );
}

#[test]
fn exact_boundaries_do_not_overlap() {
    assert_eq!(
        intersect_mix_intervals(interval(1_000, 2_000), interval(2_000, 3_000)),
        None
    );
}

#[test]
fn occupancy_windows_expand_to_the_selected_connected_episode() {
    let events = vec![
        BotConnectionEvent {
            started_ms: 100,
            completed_ms: 110,
            to_channel_id: Some(10),
            outcome: "joined".into(),
        },
        BotConnectionEvent {
            started_ms: 200,
            completed_ms: 210,
            to_channel_id: Some(10),
            outcome: "already_in_channel".into(),
        },
        BotConnectionEvent {
            started_ms: 500,
            completed_ms: 520,
            to_channel_id: Some(11),
            outcome: "switched".into(),
        },
        BotConnectionEvent {
            started_ms: 900,
            completed_ms: 910,
            to_channel_id: None,
            outcome: "disconnected".into(),
        },
        BotConnectionEvent {
            started_ms: 2_000,
            completed_ms: 2_010,
            to_channel_id: Some(10),
            outcome: "joined".into(),
        },
    ];
    let windows = build_occupancy_windows(&events, 3_000);
    assert_eq!(
        windows
            .iter()
            .map(|window| (
                window.channel_id,
                window.start_ms,
                window.end_ms,
                window.episode_id
            ))
            .collect::<Vec<_>>(),
        vec![(10, 110, 500, 1), (11, 520, 900, 1), (10, 2_010, 3_000, 2)]
    );
    let selected_channels = HashSet::from([10, 11]);
    let selected = select_occupancy_windows(windows, 300, 600, &selected_channels);
    assert_eq!(
        selected,
        vec![
            MixWindow {
                channel_id: 10,
                start_ms: 110,
                end_ms: 500,
            },
            MixWindow {
                channel_id: 11,
                start_ms: 520,
                end_ms: 900,
            },
        ]
    );
}

#[test]
fn failed_connection_audits_do_not_create_occupancy() {
    let events = vec![BotConnectionEvent {
        started_ms: 100,
        completed_ms: 110,
        to_channel_id: Some(10),
        outcome: "join_failed".into(),
    }];
    assert!(build_occupancy_windows(&events, 3_000).is_empty());
}

#[test]
fn a_repeated_anchor_visit_has_independent_overlap_windows() {
    let anchor = [interval(1_000, 2_000), interval(4_000, 5_000)];
    let contributor = interval(1_500, 4_500);
    let overlaps = anchor
        .iter()
        .filter_map(|window| intersect_mix_intervals(window.clone(), contributor.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        overlaps,
        vec![interval(1_500, 2_000), interval(4_000, 4_500)]
    );
}

#[test]
fn delay_is_relative_to_the_anchor_start_even_across_a_gap() {
    let overlap_start = 4_000;
    let anchor_start = 1_000;
    assert_eq!(overlap_start - anchor_start, 3_000);
}

#[test]
fn channel_switches_never_contribute_to_the_wrong_anchor_visit() {
    let anchor = AudioFragment {
        id: 1,
        guild_id: 1,
        channel_id: 10,
        user_id: 20,
        recording_session_id: Some(1),
        file_name: "anchor".into(),
        year: 2026,
        month: 8,
        start_ms: 1_000,
        end_ms: Some(2_000),
        segment_index: Some(0),
        live: false,
    };
    let other_channel = AudioFragment {
        channel_id: 11,
        ..anchor.clone()
    };
    let mut sources = Vec::new();
    add_fragment_sources(&mut sources, &[anchor], &[other_channel], 20, 1_000, 2_000);
    assert!(sources.is_empty());
}

#[test]
fn output_duration_includes_anchor_handoff_gaps() {
    let anchor_start = 1_000;
    let anchor_end = 9_000;
    let fragments = [interval(1_000, 3_000), interval(6_000, 9_000)];
    let covered: i64 = fragments
        .iter()
        .map(|fragment| fragment.end_ms - fragment.start_ms)
        .sum();
    assert_eq!(anchor_end - anchor_start, 8_000);
    assert_eq!(covered, 5_000);
}

#[test]
fn ffmpeg_graph_uses_timestamp_delay_mono_mix_and_limiter() {
    let plan = MixPlan {
        session_id: 1,
        scope: ChannelMixScope::SelectedSession,
        duration_ms: 1_000,
        contributors: Vec::new(),
        sources: vec![MixSource {
            audio_file_id: 2,
            recording_session_id: None,
            participant_user_id: 20,
            guild_id: 1,
            channel_id: 10,
            year: 2026,
            month: 8,
            file_name: "source".into(),
            path: PathBuf::from("source.ogg"),
            fragment_start_ms: 1_000,
            fragment_end_ms: 2_000,
            live: false,
            source_start_ms: 1_000,
            overlap_start_ms: 1_500,
            overlap_end_ms: 2_000,
            delay_ms: 500,
        }],
        participants: Vec::new(),
        tracks: Vec::new(),
        cache_dir: PathBuf::from("cache"),
        source_fingerprint: "test".into(),
        fingerprint: "test".into(),
        settings: ChannelMixGenerationSettings {
            participants: vec![ChannelMixParticipantSettings {
                user_id: "20".into(),
                gain_db: -6.0,
                muted: false,
            }],
            source_fingerprint: None,
        },
    };
    let filter = build_mix_filter(&plan);
    assert!(filter.contains("volume=-6dB, adelay="));
    assert!(filter.contains("atrim=start=0.500:end=1.000"));
    assert!(filter.contains("adelay=24000S|24000S"));
    assert!(filter.contains("amix=inputs=1:duration=longest:normalize=0"));
    assert!(filter.contains("alimiter=limit=0.95"));
    assert!(filter.contains("apad=whole_len=48000"));
    assert!(filter.contains("atrim=duration=1.000"));

    let muted = plan.with_settings(ChannelMixGenerationSettings {
        participants: vec![ChannelMixParticipantSettings {
            user_id: "20".into(),
            gain_db: -6.0,
            muted: true,
        }],
        source_fingerprint: None,
    });
    assert!(muted.renderable_sources().is_empty());
}

#[tokio::test]
async fn cached_mix_requires_the_current_source_fingerprint() {
    let directory = tempfile::tempdir().expect("temporary mix directory");
    let output = directory.path().join(MIX_OUTPUT);
    let fingerprint = directory.path().join(MIX_FINGERPRINT);
    tokio::fs::write(&output, b"ogg")
        .await
        .expect("write output");
    tokio::fs::write(&fingerprint, "current")
        .await
        .expect("write fingerprint");
    tokio::fs::write(
        directory.path().join(MIX_SETTINGS),
        serde_json::to_vec(&MixCacheMetadata {
            source_fingerprint: "current".into(),
            fingerprint: "current".into(),
            settings: ChannelMixGenerationSettings {
                participants: Vec::new(),
                source_fingerprint: None,
            },
        })
        .expect("serialize settings"),
    )
    .await
    .expect("write settings");
    let plan = MixPlan {
        session_id: 1,
        scope: ChannelMixScope::SelectedSession,
        duration_ms: 1_000,
        contributors: Vec::new(),
        sources: Vec::new(),
        participants: Vec::new(),
        tracks: Vec::new(),
        cache_dir: directory.path().to_path_buf(),
        source_fingerprint: "current".into(),
        fingerprint: "current".into(),
        settings: ChannelMixGenerationSettings {
            participants: Vec::new(),
            source_fingerprint: None,
        },
    };
    assert!(cache_is_valid(&plan).await);
    let stale = MixPlan {
        fingerprint: "stale".into(),
        ..plan
    };
    assert!(!cache_is_valid(&stale).await);
}

#[tokio::test]
async fn ffmpeg_fixture_starts_delayed_audio_at_the_expected_offset() {
    let Ok(version) = tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
    else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let directory = tempfile::tempdir().expect("temporary mix directory");
    let source = directory.path().join("source.ogg");
    let generated = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000:duration=1",
            "-ar",
            "48000",
            "-ac",
            "1",
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
        ])
        .arg(&source)
        .output()
        .await
        .expect("generate fixture");
    assert!(generated.status.success());

    let output = directory.path().join("combined.ogg");
    let job = Arc::new(Mutex::new(MixJob {
        source_fingerprint: "fixture".into(),
        settings: ChannelMixGenerationSettings {
            participants: Vec::new(),
            source_fingerprint: None,
        },
        progress: 0,
        failed: None,
    }));
    let plan = MixPlan {
        session_id: 1,
        scope: ChannelMixScope::SelectedSession,
        duration_ms: 1_000,
        contributors: Vec::new(),
        sources: vec![MixSource {
            audio_file_id: 2,
            recording_session_id: None,
            participant_user_id: 20,
            guild_id: 1,
            channel_id: 10,
            year: 2026,
            month: 8,
            file_name: "source".into(),
            path: source,
            fragment_start_ms: 0,
            fragment_end_ms: 1_000,
            live: false,
            source_start_ms: 0,
            overlap_start_ms: 0,
            overlap_end_ms: 1_000,
            delay_ms: 500,
        }],
        participants: Vec::new(),
        tracks: Vec::new(),
        cache_dir: directory.path().to_path_buf(),
        source_fingerprint: "fixture".into(),
        fingerprint: "fixture".into(),
        settings: ChannelMixGenerationSettings {
            participants: Vec::new(),
            source_fingerprint: None,
        },
    };
    run_mix_ffmpeg(&plan, &job, &output)
        .await
        .expect("render delayed fixture");

    let probe = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(&output)
        .output()
        .await
        .expect("probe fixture");
    let duration = String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse::<f64>()
        .expect("fixture duration");
    assert!((0.98..=1.03).contains(&duration), "duration={duration}");

    let silence = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostats",
            "-i",
            output.to_str().expect("output path"),
            "-af",
            "silencedetect=n=-45dB:d=0.1",
            "-f",
            "null",
            "-",
        ])
        .output()
        .await
        .expect("inspect fixture");
    let stderr = String::from_utf8_lossy(&silence.stderr);
    let silence_end = stderr
        .split("silence_end: ")
        .nth(1)
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<f64>().ok())
        .expect("silence end");
    assert!(
        (0.4..=0.6).contains(&silence_end),
        "silence_end={silence_end}"
    );
}
