use lucas_app::{
    calibration::{CalibrationScript, fit_session, observation},
    runtime::{CalibrationState, Inputs, Runtime, Status, camera::CaptureSession},
    timer::alignment_store::AlignmentStore,
};
use lucas_core::DeathClassification;
use lucas_game::{Event, EventData, GameStats, Player, Snapshot};
use lucas_gaze::{
    GazeFeatures, GazeSample, GazeSource, calibration::CalibrationModel, source::TraceGazeSource,
};
use lucas_store::{Settings, Store, StoredEventKind};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn sample(at: u64, p: [f64; 2]) -> GazeSample {
    GazeSample {
        timestamp: Duration::from_millis(at),
        quality: 0.9,
        features: Some(GazeFeatures::from_array([
            p[0] as f32,
            p[1] as f32,
            0.2,
            p[0] as f32,
            p[1] as f32,
            0.2,
            0.0,
            0.0,
            0.0,
            3.0,
        ])),
    }
}
fn fitted() -> CalibrationModel {
    let script = CalibrationScript::new(
        [1920, 1080],
        Default::default(),
        &AlignmentStore::default(),
        [531.0, 299.0],
        650.0,
    )
    .unwrap();
    let observations: Vec<_> = script
        .targets()
        .iter()
        .flat_map(|t| (0..30).map(move |i| observation(t.target, sample(i, t.target.position))))
        .collect();
    fit_session(&script, &observations).unwrap()
}
fn event(id: u64, time: f64, name: &str) -> Event {
    Event {
        event_id: id,
        event_name: name.into(),
        event_time: time,
        victim_name: Some("Player#EUW".into()),
        killer_name: None,
        assisters: vec![],
        dragon_type: None,
        turret_killed: None,
        result: None,
    }
}
fn snapshot(time: f64, dead: bool, events: Vec<Event>) -> Snapshot {
    Snapshot {
        stats: GameStats {
            game_mode: "CLASSIC".into(),
            game_time: time,
            map_name: "Map11".into(),
            map_number: 11,
        },
        active_player_name: "Player#EUW".into(),
        players: vec![Player {
            riot_id: "Player#EUW".into(),
            riot_id_game_name: String::new(),
            riot_id_tag_line: String::new(),
            summoner_name: String::new(),
            champion_name: "Ahri".into(),
            team: "ORDER".into(),
            is_dead: dead,
            respawn_timer: if dead { 2.0 } else { 0.0 },
        }],
        events: EventData { events },
    }
}
fn inputs() -> Inputs {
    Inputs {
        window_exists: true,
        foreground: true,
        geometry_ready: true,
        gaze_mode: true,
        paused: false,
    }
}
fn ready(runtime: &mut Runtime, model: CalibrationModel) {
    runtime.install_calibration(Some(model)).unwrap();
}

#[test]
fn replay_checks_pulses_unknown_spans_and_stored_blind_seen_unknown_deaths() {
    let model = fitted();
    let [l, t, r, b] = model.screen().minimap;
    let map = [(l + r) / 2.0, (t + b) / 2.0];
    let frames = (0..=1200).map(|i| {
        let ms = 1 + i * 50;
        let mut s = sample(
            ms,
            if (1000..1300).contains(&ms)
                || (20000..20500).contains(&ms)
                || (55000..55500).contains(&ms)
            {
                map
            } else {
                [0.5, 0.5]
            },
        );
        if (30000..35000).contains(&ms) {
            s.features = None;
            s.quality = 0.0;
        }
        if (35000..41000).contains(&ms) {
            s = sample(ms, [2.0, 2.0]);
        }
        s
    });
    let mut source = TraceGazeSource::new(frames);
    let mut runtime = Runtime::new(Settings {
        density_cap: false,
        ..Settings::default()
    })
    .unwrap();
    let mut pulses = Vec::new();
    let mut unknown_gap = None;
    let mut checks = Vec::new();
    let mut timeline = Vec::new();
    while let Some(s) = source.next_sample().unwrap() {
        let ms = s.timestamp.as_millis() as u64;
        if (ms - 1).is_multiple_of(1000) {
            let time = 90.0 + ms as f64 / 1000.0;
            let events = [(14.0, 1), (22.0, 2), (40.0, 3)]
                .into_iter()
                .filter(|(t, _)| time >= 90.0 + t)
                .map(|(t, id)| event(id, 90.0 + t, "ChampionKill"))
                .collect();
            let dead = (14000..16000).contains(&ms)
                || (22000..24000).contains(&ms)
                || (40000..42000).contains(&ms);
            runtime
                .poll(
                    s.timestamp,
                    Some(&snapshot(time, dead, events)),
                    1_000_000 + ms,
                )
                .unwrap();
            if ms == 1 {
                ready(&mut runtime, model.clone());
            }
        }
        let tick = runtime
            .tick(
                s.timestamp,
                Inputs {
                    foreground: !(45000..47000).contains(&ms),
                    ..inputs()
                },
                Some(s),
                1_000_000 + ms,
            )
            .unwrap();
        if (ms - 1).is_multiple_of(1000) || tick.pulse.is_some() {
            timeline.push(serde_json::json!({
                "elapsed_ms": ms,
                "status": format!("{:?}", tick.status),
                "eligible_gap_ms": tick.gap_ms,
                "overlay_hidden": tick.hide,
                "pulse": tick.pulse.is_some(),
            }));
        }
        if tick.pulse.is_some() {
            pulses.push(ms);
        }
        if (30100..39000).contains(&ms) {
            assert_eq!(tick.status, Status::TrackingLost);
            assert!(tick.hide);
            assert_eq!(*unknown_gap.get_or_insert(tick.gap_ms), tick.gap_ms);
        }
        if tick.hide && tick.status == Status::Active {
            checks.push(ms);
        }
    }
    runtime
        .poll(
            Duration::from_millis(61001),
            Some(&snapshot(151.001, false, vec![event(4, 151.0, "GameEnd")])),
            1_061_001,
        )
        .unwrap();
    assert!(!runtime.camera_wanted(true, true, false));
    let summaries = runtime.take_summaries();
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary.metrics.checks, 3);
    assert!(!checks.is_empty());
    assert_eq!(
        (
            summary.metrics.blind_deaths,
            summary.metrics.seen_deaths,
            summary.metrics.unknown_deaths
        ),
        (1, 1, 1)
    );
    assert_eq!(summary.context.champion.as_deref(), Some("Ahri"));
    assert!(summary.context.role.is_none());
    assert!(summary.context.tracking.valid_fraction().unwrap() < 0.9);
    assert!(summary.metrics.eligible_ms < 48000);
    assert_eq!(pulses.len(), 2, "{pulses:?}");
    assert!((10300..12400).contains(&pulses[0]), "{pulses:?}");
    assert!((53000..55100).contains(&pulses[1]), "{pulses:?}");
    let deaths: Vec<_> = summary
        .events
        .iter()
        .filter_map(|e| {
            if let StoredEventKind::Death { classification, .. } = e.kind {
                Some((e.game_time_ms, classification))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        deaths,
        vec![
            (104000, DeathClassification::Blind),
            (112000, DeathClassification::Seen),
            (130000, DeathClassification::Unknown)
        ]
    );
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in(dir.path()).unwrap();
    store.save_summary(summary).unwrap();
    drop(store);
    assert_eq!(
        Store::open_in(dir.path())
            .unwrap()
            .summary(&summary.id)
            .unwrap()
            .as_ref(),
        Some(summary)
    );
    assert!(!serde_json::to_string(summary).unwrap().contains("features"));

    // Opt-in evidence from the executed runtime, not a hand-authored demo.
    // The native --show-postgame CLI can open this exact persisted game.
    if let Some(path) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let directory = std::path::Path::new(&path);
        std::fs::create_dir_all(directory).unwrap();
        let fixture = directory.join("runtime-replay-store");
        assert!(!fixture.exists(), "refusing to overwrite evidence");
        let mut store = Store::open_in(&fixture).unwrap();
        store.save_summary(summary).unwrap();
        drop(store);
        let persisted = Store::open_in(&fixture)
            .unwrap()
            .summary(&summary.id)
            .unwrap()
            .unwrap();
        assert_eq!(&persisted, summary);
        std::fs::write(
            directory.join("runtime-replay-summary.json"),
            serde_json::to_vec_pretty(&persisted).unwrap(),
        )
        .unwrap();
        std::fs::write(
            directory.join("runtime-replay-timeline.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "synthetic_gaze_and_api_not_native_webcam_acceptance": true,
                "scenario": "Minimap dwells at 1, 20 and 55 s; deaths at 14, 22 and 40 s; no face at 30–35 s; off-screen at 35–41 s; foreground lost at 45–47 s; GameEnd at 61 s",
                "pulse_elapsed_ms": pulses,
                "camera_wanted_after_game_end": runtime.camera_wanted(true, true, false),
                "postgame_ready": runtime.postgame_ready(),
                "ticks": timeline,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn five_second_repeats_hide_on_check_restart_after_view_and_survive_storage() {
    let model = fitted();
    let [l, t, r, b] = model.screen().minimap;
    let map = [(l + r) / 2.0, (t + b) / 2.0];
    let mut runtime = Runtime::new(Settings {
        level_index: 4,
        density_cap: true,
        ..Settings::default()
    })
    .unwrap();
    let mut pulses = Vec::new();
    for ms in (0..=26_000).step_by(50) {
        let now = Duration::from_millis(ms);
        if ms % 1000 == 0 {
            runtime
                .poll(
                    now,
                    Some(&snapshot(100.0 + ms as f64 / 1000.0, false, vec![])),
                    1_000_000 + ms,
                )
                .unwrap();
            if ms == 0 {
                ready(&mut runtime, model.clone());
            }
        }
        let looking = (17_000..19_000).contains(&ms);
        let tick = runtime
            .tick(
                now,
                inputs(),
                Some(sample(ms, if looking { map } else { [0.5, 0.5] })),
                1_000_000 + ms,
            )
            .unwrap();
        if let Some(pulse) = tick.pulse {
            assert!(!tick.hide);
            assert!((4_500..=5_500).contains(&pulse.effective_delay_ms));
            pulses.push(ms);
        }
        if (17_100..19_000).contains(&ms) {
            assert!(tick.hide);
            assert_eq!(tick.gap_ms, 0);
            assert!(tick.pulse.is_none());
        }
    }
    assert_eq!(pulses.len(), 4, "{pulses:?}");
    assert!((4_500..=5_550).contains(&pulses[0]));
    for pair in pulses[..3].windows(2) {
        assert!((4_500..=5_550).contains(&(pair[1] - pair[0])));
    }
    assert!((23_500..=24_550).contains(&pulses[3]));
    runtime
        .finish(Duration::from_millis(26_000), 1_026_000)
        .unwrap();
    let summaries = runtime.take_summaries();
    let summary = &summaries[0];
    assert_eq!(summary.metrics.pulses, 4);
    assert_eq!(summary.metrics.checks, 1);
    assert_eq!(
        summary
            .events
            .iter()
            .filter(|e| matches!(e.kind, StoredEventKind::Pulse))
            .count(),
        4
    );
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in(dir.path()).unwrap();
    store.save_summary(summary).unwrap();
    assert_eq!(store.summary(&summary.id).unwrap().as_ref(), Some(summary));
}

#[test]
fn failed_poll_makes_outage_deaths_unknown_but_preserves_earlier_evidence() {
    let model = fitted();
    let [l, t, r, b] = model.screen().minimap;
    let map = [(l + r) / 2.0, (t + b) / 2.0];
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    let deaths = vec![
        event(1, 108.5, "ChampionKill"),
        event(2, 110.25, "ChampionKill"),
    ];
    for ms in (0..=12000).step_by(50) {
        let now = Duration::from_millis(ms);
        if ms % 1000 == 0 {
            let poll = match ms {
                11000 => None,
                12000 => Some(snapshot(111.0, false, deaths.clone())),
                _ => Some(snapshot(100.0 + ms as f64 / 1000.0, false, vec![])),
            };
            runtime.poll(now, poll.as_ref(), ms).unwrap();
            if ms == 0 {
                ready(&mut runtime, model.clone());
            }
        }
        let position = if (1000..1300).contains(&ms) {
            map
        } else {
            [0.5, 0.5]
        };
        runtime
            .tick(now, inputs(), Some(sample(ms, position)), ms)
            .unwrap();
    }
    let mut events = deaths;
    events.push(event(3, 112.0, "GameEnd"));
    runtime
        .poll(
            Duration::from_secs(13),
            Some(&snapshot(112.0, false, events)),
            13000,
        )
        .unwrap();
    let summaries = runtime.take_summaries();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].metrics.seen_deaths, 1);
    assert_eq!(summaries[0].metrics.blind_deaths, 0);
    assert_eq!(summaries[0].metrics.unknown_deaths, 1);
    assert!(summaries[0].events.iter().any(|e| e.game_time_ms == 110250
        && matches!(
            e.kind,
            StoredEventKind::Death {
                classification: DeathClassification::Unknown,
                ..
            }
        )));
}

#[test]
fn calibration_is_needed_only_without_a_usable_model_and_survives_each_game() {
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    runtime
        .poll(
            Duration::from_millis(1),
            Some(&snapshot(100.0, false, vec![])),
            1,
        )
        .unwrap();
    assert_eq!(
        runtime
            .tick(
                Duration::from_millis(1),
                inputs(),
                Some(sample(1, [0.5, 0.5])),
                1
            )
            .unwrap()
            .status,
        Status::CalibrationNeeded
    );
    runtime.install_calibration(Some(fitted())).unwrap();
    assert_eq!(runtime.calibration(), CalibrationState::Ready);
    runtime
        .poll(
            Duration::from_millis(1001),
            Some(&snapshot(101.0, false, vec![event(1, 101.0, "GameEnd")])),
            1001,
        )
        .unwrap();
    runtime
        .poll(
            Duration::from_millis(2001),
            Some(&snapshot(0.0, false, vec![])),
            2001,
        )
        .unwrap();
    assert_eq!(runtime.calibration(), CalibrationState::Ready);
    runtime.install_calibration(None).unwrap(); // changed screen/alignment
    assert_eq!(runtime.calibration(), CalibrationState::Needed);
}

#[test]
fn calibration_windows_pause_gaze_without_revoking_the_saved_fit() {
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    runtime
        .poll(Duration::ZERO, Some(&snapshot(100.0, false, vec![])), 0)
        .unwrap();
    ready(&mut runtime, fitted());
    runtime.reset_gaze().unwrap();
    assert_eq!(runtime.calibration(), CalibrationState::Ready);
    for ms in 1..=300 {
        let tick = runtime
            .tick(
                Duration::from_millis(ms),
                Inputs {
                    paused: true,
                    ..inputs()
                },
                Some(sample(ms, [0.5, 0.5])),
                ms,
            )
            .unwrap();
        assert_eq!(tick.status, Status::Paused);
        assert!(tick.hide);
        assert!(tick.pulse.is_none());
        assert_eq!(tick.gap_ms, 0);
    }
    runtime.reset_gaze().unwrap();
    assert_eq!(runtime.calibration(), CalibrationState::Ready);
    let resumed = runtime
        .tick(
            Duration::from_millis(301),
            inputs(),
            Some(sample(301, [0.5, 0.5])),
            301,
        )
        .unwrap();
    assert_eq!(resumed.status, Status::Active);
    assert_eq!(resumed.gap_ms, 0);
    assert!(resumed.pulse.is_none());
}

#[test]
fn missing_calibration_stops_camera_without_stopping_session_ticks() {
    let model = fitted();
    for live_client_gate in [true, false] {
        for recalibrating in [true, false] {
            let mut runtime = Runtime::new(Settings {
                live_client_gate,
                ..Settings::default()
            })
            .unwrap();
            runtime
                .poll(Duration::ZERO, Some(&snapshot(100.0, false, vec![])), 0)
                .unwrap();
            runtime.tick(Duration::ZERO, inputs(), None, 0).unwrap();
            assert!(runtime.in_session());
            assert!(!runtime.camera_wanted(true, true, false));
            ready(&mut runtime, model.clone());
            assert!(runtime.camera_wanted(true, true, false));

            // Missing or invalidated calibration is still a hard camera gate.
            // A calibration window separately pauses a valid model.
            if recalibrating {
                runtime.reset_gaze().unwrap();
            }
            runtime.install_calibration(None).unwrap();
            for ms in 1..=3 {
                assert!(!runtime.camera_wanted(true, true, false));
                let tick = runtime
                    .tick(Duration::from_millis(ms), inputs(), None, ms)
                    .unwrap();
                assert_eq!(tick.status, Status::CalibrationNeeded);
                assert!(tick.hide);
                assert!(tick.pulse.is_none());
                assert!(runtime.in_session());
            }

            runtime.install_calibration(Some(model.clone())).unwrap();
            assert!(runtime.camera_wanted(true, true, false));
            assert!(!runtime.camera_wanted(true, true, true));
            assert_eq!(
                runtime
                    .tick(
                        Duration::from_millis(4),
                        inputs(),
                        Some(sample(4, [0.5, 0.5])),
                        4
                    )
                    .unwrap()
                    .status,
                Status::Active
            );
            assert_eq!(runtime.calibration(), CalibrationState::Ready);
            assert!(runtime.camera_wanted(true, true, false));
        }
    }
}

#[test]
fn reconnect_after_window_loss_reuses_calibration_without_replaying_deaths() {
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    let death = event(1, 99.0, "ChampionKill");
    runtime
        .poll(
            Duration::ZERO,
            Some(&snapshot(100.0, false, vec![death.clone()])),
            0,
        )
        .unwrap();
    ready(&mut runtime, fitted());
    runtime.poll(Duration::from_secs(1), None, 1000).unwrap();
    runtime
        .tick(Duration::from_secs(1), Inputs::default(), None, 1000)
        .unwrap();
    assert!(!runtime.in_session());
    assert!(runtime.postgame_ready());
    assert_eq!(runtime.take_summaries().len(), 1);

    runtime
        .poll(
            Duration::from_secs(2),
            Some(&snapshot(102.0, false, vec![death.clone()])),
            2000,
        )
        .unwrap();
    assert!(runtime.in_session());
    assert!(!runtime.postgame_ready());
    assert!(runtime.camera_wanted(true, true, false));
    assert_eq!(runtime.calibration(), CalibrationState::Ready);
    let tick = runtime
        .tick(
            Duration::from_secs(2),
            inputs(),
            Some(sample(2000, [0.5, 0.5])),
            2000,
        )
        .unwrap();
    assert_eq!(tick.status, Status::Active);
    assert!(!tick.hide);
    runtime
        .poll(
            Duration::from_secs(3),
            Some(&snapshot(
                103.0,
                false,
                vec![death, event(2, 103.0, "GameEnd")],
            )),
            3000,
        )
        .unwrap();
    let summaries = runtime.take_summaries();
    assert_eq!(summaries.len(), 1);
    assert!(
        !summaries[0]
            .events
            .iter()
            .any(|e| matches!(e.kind, StoredEventKind::Death { .. }))
    );
    runtime
        .poll(
            Duration::from_secs(4),
            Some(&snapshot(104.0, false, vec![event(2, 103.0, "GameEnd")])),
            4000,
        )
        .unwrap();
    assert!(!runtime.in_session());
    assert!(runtime.take_summaries().is_empty());
}

#[test]
fn gate_transitions_defer_postgame_until_window_loss_or_game_end() {
    for reconnect in [false, true] {
        let mut runtime = Runtime::new(Settings {
            live_client_gate: false,
            ..Settings::default()
        })
        .unwrap();
        runtime.tick(Duration::ZERO, inputs(), None, 0).unwrap();
        runtime
            .set_live_gate(true, Duration::from_secs(1), 1000)
            .unwrap();
        assert!(!runtime.in_session());
        assert_eq!(runtime.take_summaries().len(), 1);
        assert!(!runtime.postgame_ready());
        runtime.poll(Duration::from_secs(2), None, 2000).unwrap();
        runtime
            .tick(Duration::from_secs(2), inputs(), None, 2000)
            .unwrap();
        assert!(!runtime.postgame_ready());
        if reconnect {
            runtime
                .poll(
                    Duration::from_secs(3),
                    Some(&snapshot(103.0, false, vec![])),
                    3000,
                )
                .unwrap();
            assert!(!runtime.postgame_ready());
            runtime
                .poll(
                    Duration::from_secs(4),
                    Some(&snapshot(104.0, false, vec![event(1, 104.0, "GameEnd")])),
                    4000,
                )
                .unwrap();
        } else {
            runtime
                .tick(Duration::from_secs(3), Inputs::default(), None, 3000)
                .unwrap();
        }
        assert!(runtime.postgame_ready());
    }
}

#[test]
fn unknown_dwell_stale_frames_offscreen_closed_eyes_and_gate_pauses() {
    let model = fitted();
    let [l, t, r, b] = model.screen().minimap;
    let map = [(l + r) / 2.0, (t + b) / 2.0];
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    runtime
        .poll(
            Duration::from_millis(1),
            Some(&snapshot(100.0, false, vec![])),
            1,
        )
        .unwrap();
    ready(&mut runtime, model);
    let first = sample(1, map);
    for ms in [1, 51, 101, 151] {
        // Replaying a single frame cannot complete a dwell.
        let tick = runtime
            .tick(Duration::from_millis(ms), inputs(), Some(first), ms)
            .unwrap();
        assert!(!tick.hide);
    }
    let tick = runtime
        .tick(Duration::from_millis(301), inputs(), None, 301)
        .unwrap();
    assert_eq!(tick.status, Status::TrackingLost);
    let gap = tick.gap_ms;
    assert_eq!(
        gap, 250,
        "camera expiry is measured from acquisition, not the last tick"
    );
    for (i, mut s) in [sample(401, [2.0, 2.0]), sample(501, map), sample(601, map)]
        .into_iter()
        .enumerate()
    {
        if i == 1 {
            s.quality = 0.1;
        }
        if i == 2 {
            let mut f = s.features.unwrap().as_array();
            f[2] = 0.0;
            f[5] = 0.0;
            s.features = Some(GazeFeatures::from_array(f));
        }
        let tick = runtime.tick(s.timestamp, inputs(), Some(s), 0).unwrap();
        assert_eq!(tick.status, Status::TrackingLost);
        assert_eq!(tick.gap_ms, gap);
    }
    for input in [
        Inputs {
            foreground: false,
            ..inputs()
        },
        Inputs {
            paused: true,
            ..inputs()
        },
        Inputs {
            geometry_ready: false,
            ..inputs()
        },
        Inputs {
            gaze_mode: false,
            ..inputs()
        },
    ] {
        let now = Duration::from_millis(701);
        assert!(
            runtime
                .tick(now, input, Some(sample(701, map)), 0)
                .unwrap()
                .hide
        );
    }
    runtime.finish(Duration::from_millis(801), 1000).unwrap();
    assert_eq!(runtime.take_summaries()[0].metrics.checks, 0);
}

struct CountedSource(Arc<AtomicUsize>);
impl Drop for CountedSource {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl GazeSource for CountedSource {
    fn next_sample(&mut self) -> lucas_gaze::Result<Option<GazeSample>> {
        Ok(Some(GazeSample {
            timestamp: Duration::ZERO,
            features: None,
            quality: 0.0,
        }))
    }
}
#[test]
fn camera_lifetime_start_loss_reconnect_calibration_end_and_exit() {
    let opens = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let mut capture = CaptureSession::default();
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    let sync = |capture: &mut CaptureSession<CountedSource>, runtime: &Runtime, calibrating| {
        capture
            .set_wanted(runtime.camera_wanted(true, true, calibrating), || {
                opens.fetch_add(1, Ordering::SeqCst);
                Ok(CountedSource(drops.clone()))
            })
            .unwrap();
    };
    sync(&mut capture, &runtime, false);
    assert_eq!(opens.load(Ordering::SeqCst), 0);
    runtime
        .poll(
            Duration::from_millis(1),
            Some(&snapshot(0.0, false, vec![])),
            1,
        )
        .unwrap();
    sync(&mut capture, &runtime, false);
    assert_eq!(opens.load(Ordering::SeqCst), 0);
    runtime.install_calibration(Some(fitted())).unwrap();
    sync(&mut capture, &runtime, false);
    sync(&mut capture, &runtime, false);
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    capture.next_sample().unwrap(); // missing face doesn't release a running game's device
    sync(&mut capture, &runtime, true);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    sync(&mut capture, &runtime, false);
    runtime
        .poll(Duration::from_millis(1001), None, 1001)
        .unwrap();
    sync(&mut capture, &runtime, false);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert!(runtime.take_summaries().is_empty()); // no focus-stealing card on transient outage
    runtime
        .poll(
            Duration::from_millis(2001),
            Some(&snapshot(2.0, false, vec![])),
            2001,
        )
        .unwrap();
    sync(&mut capture, &runtime, false);
    runtime
        .poll(
            Duration::from_millis(3001),
            Some(&snapshot(3.0, false, vec![event(1, 3.0, "GameEnd")])),
            3001,
        )
        .unwrap();
    sync(&mut capture, &runtime, false);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
    runtime
        .poll(
            Duration::from_millis(4001),
            Some(&snapshot(0.0, false, vec![])),
            4001,
        )
        .unwrap();
    sync(&mut capture, &runtime, false);
    drop(capture);
    assert_eq!(opens.load(Ordering::SeqCst), drops.load(Ordering::SeqCst));
}

#[test]
fn every_game_gate_fails_closed_and_timer_help_is_still_assistance() {
    let model = fitted();
    for case in 0..6 {
        let mut runtime = Runtime::new(Settings::default()).unwrap();
        let mut game = snapshot(100.0, false, vec![]);
        match case {
            0 => game.stats.game_time = 90.0,
            1 => game.players[0].is_dead = true,
            2 => game.stats.map_number = 12,
            3 => game.stats.game_mode = "PRACTICETOOL".into(),
            _ => {}
        }
        runtime
            .poll(Duration::from_millis(1), Some(&game), 1)
            .unwrap();
        ready(&mut runtime, model.clone());
        let ms = if case == 4 {
            2101
        } else if case == 5 {
            3101
        } else {
            1
        };
        let tick = runtime
            .tick(
                Duration::from_millis(ms),
                inputs(),
                Some(sample(ms, [0.5, 0.5])),
                ms,
            )
            .unwrap();
        assert_eq!(tick.status, Status::Paused, "gate {case}");
        assert!(tick.hide && tick.pulse.is_none());
        assert_eq!(tick.gap_ms, 0);
    }
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    runtime
        .poll(
            Duration::from_millis(1),
            Some(&snapshot(100.0, false, vec![])),
            1,
        )
        .unwrap();
    runtime
        .tick(
            Duration::from_millis(1),
            Inputs {
                gaze_mode: false,
                ..inputs()
            },
            None,
            1,
        )
        .unwrap();
    runtime.timer_pulse(Duration::from_millis(1)).unwrap();
    runtime.finish(Duration::from_millis(2), 2).unwrap();
    assert_eq!(runtime.take_summaries()[0].metrics.pulses, 1);
}

#[test]
fn historical_deaths_and_window_only_summary_do_not_invent_gaze_or_death_data() {
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    runtime
        .poll(
            Duration::from_millis(1),
            Some(&snapshot(
                120.0,
                false,
                vec![event(1, 100.0, "ChampionKill")],
            )),
            1,
        )
        .unwrap();
    runtime.finish(Duration::from_millis(2), 2).unwrap();
    let summary = runtime.take_summaries().remove(0);
    assert_eq!(summary.metrics.unknown_deaths, 1);
    assert_eq!(summary.events[0].game_time_ms, 100000);
    let mut runtime = Runtime::new(Settings {
        live_client_gate: false,
        ..Settings::default()
    })
    .unwrap();
    runtime
        .tick(Duration::from_millis(1), inputs(), None, 1)
        .unwrap();
    ready(&mut runtime, fitted());
    runtime
        .tick(
            Duration::from_millis(1001),
            inputs(),
            Some(sample(1001, [0.5, 0.5])),
            1001,
        )
        .unwrap();
    runtime
        .tick(Duration::from_millis(2001), Inputs::default(), None, 2001)
        .unwrap();
    let summary = runtime.take_summaries().remove(0);
    assert!(!summary.context.deaths_available);
    assert_eq!(summary.metrics.blind_deaths, 0);
}

#[test]
fn saved_level_waits_for_next_match_even_across_gate_changes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path()).unwrap();
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    runtime
        .poll(Duration::ZERO, Some(&snapshot(100.0, false, vec![])), 0)
        .unwrap();
    let initial = runtime.active_level().unwrap();
    let mut settings = runtime.settings().clone();
    settings.level_index = lucas_core::TRAINING_LEVELS.len() - 2;
    store.save_settings(&settings).unwrap();
    runtime
        .update_preferences(&store.load_settings().unwrap())
        .unwrap();
    assert_eq!(runtime.active_level(), Some(initial));
    assert_eq!(
        Runtime::new(store.load_settings().unwrap())
            .unwrap()
            .settings()
            .level_index,
        settings.level_index
    );
    // Switching gate is not permission to change the level mid-match.
    runtime
        .set_live_gate(false, Duration::from_secs(1), 1000)
        .unwrap();
    runtime
        .tick(Duration::from_secs(1), inputs(), None, 1000)
        .unwrap();
    assert_eq!(runtime.active_level(), Some(initial));
    runtime
        .tick(Duration::from_secs(2), Inputs::default(), None, 2000)
        .unwrap();
    runtime
        .tick(Duration::from_secs(3), inputs(), None, 3000)
        .unwrap();
    assert_eq!(
        runtime.active_level(),
        Some(lucas_core::TRAINING_LEVELS[settings.level_index])
    );
    assert!(
        runtime
            .take_summaries()
            .iter()
            .all(|s| s.initial_level == initial)
    );
}

#[test]
fn solo_game_measures_checks_and_deaths_stores_probe_and_resets_for_next_game() {
    let model = fitted();
    let [l, t, r, b] = model.screen().minimap;
    let map = [(l + r) / 2.0, (t + b) / 2.0];
    let mut runtime = Runtime::new(Settings {
        density_cap: false,
        ..Settings::default()
    })
    .unwrap();
    runtime.arm_solo(true);
    runtime.arm_solo(false);
    assert!(!runtime.solo_armed());
    runtime.arm_solo(true);
    assert!(runtime.solo_armed());
    for i in 0..=500 {
        let ms = 1 + i * 50;
        let at = Duration::from_millis(ms);
        if i % 20 == 0 {
            let deaths = if ms >= 13000 {
                vec![event(1, 102.0, "ChampionKill")]
            } else {
                vec![]
            };
            runtime
                .poll(
                    at,
                    Some(&snapshot(90.0 + ms as f64 / 1000.0, false, deaths)),
                    ms,
                )
                .unwrap();
            if i == 0 {
                ready(&mut runtime, model.clone());
            }
        }
        assert!(runtime.solo_active());
        assert!(!runtime.solo_armed());
        let tick = runtime
            .tick(
                at,
                inputs(),
                Some(sample(
                    ms,
                    if (1000..1500).contains(&ms) || (20000..20500).contains(&ms) {
                        map
                    } else {
                        [0.5, 0.5]
                    },
                )),
                ms,
            )
            .unwrap();
        assert!(tick.hide);
        assert!(tick.pulse.is_none());
        runtime.timer_pulse(at).unwrap(); // timer mode cannot contaminate a probe
    }
    runtime
        .poll(
            Duration::from_secs(26),
            Some(&snapshot(
                116.0,
                false,
                vec![event(1, 102.0, "ChampionKill"), event(2, 116.0, "GameEnd")],
            )),
            26000,
        )
        .unwrap();
    assert!(!runtime.solo_active());
    assert!(!runtime.solo_armed());
    let summary = runtime.take_summaries().remove(0);
    assert!(summary.context.probe_game);
    assert_eq!(summary.metrics.pulses, 0);
    assert!(summary.metrics.eligible_ms > 20000);
    assert!(
        summary
            .events
            .iter()
            .any(|e| matches!(e.kind, StoredEventKind::Check))
    );
    assert_eq!(summary.metrics.blind_deaths, 1);
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open_in(dir.path()).unwrap();
    store.save_summary(&summary).unwrap();
    let stored = store.summary(&summary.id).unwrap().unwrap();
    assert!(stored.context.probe_game);
    assert_eq!(stored.metrics.pulses, 0);
    // The native --show-postgame CLI can display this exact replay result.
    // Synthetic samples exercise the real runtime/store, not webcam accuracy.
    if let Some(path) = std::env::var_os("LUCAS_TEST_EVIDENCE_DIR") {
        let directory = std::path::Path::new(&path);
        std::fs::create_dir_all(directory).unwrap();
        let fixture = directory.join("solo-replay-store");
        assert!(!fixture.exists(), "refusing to overwrite evidence");
        let mut evidence_store = Store::open_in(&fixture).unwrap();
        evidence_store.save_summary(&stored).unwrap();
        drop(evidence_store);
        let persisted = Store::open_in(&fixture)
            .unwrap()
            .summary(&stored.id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted, stored);
        std::fs::write(
            directory.join("solo-replay-summary.json"),
            serde_json::to_vec_pretty(&persisted).unwrap(),
        )
        .unwrap();
    }
    runtime
        .poll(
            Duration::from_secs(30),
            Some(&snapshot(0.0, false, vec![])),
            30000,
        )
        .unwrap();
    assert!(runtime.in_session());
    assert!(!runtime.solo_active());
}

#[test]
fn arming_during_play_only_affects_next_match_and_solo_survives_gate_switch() {
    let mut runtime = Runtime::new(Settings::default()).unwrap();
    runtime
        .poll(Duration::ZERO, Some(&snapshot(100.0, false, vec![])), 0)
        .unwrap();
    runtime.arm_solo(true);
    assert!(!runtime.solo_active());
    runtime
        .set_live_gate(false, Duration::from_secs(1), 1000)
        .unwrap();
    runtime
        .tick(Duration::from_secs(1), inputs(), None, 1000)
        .unwrap();
    assert!(!runtime.solo_active());
    assert!(runtime.solo_armed());
    runtime
        .tick(Duration::from_secs(2), Inputs::default(), None, 2000)
        .unwrap();
    runtime
        .tick(Duration::from_secs(3), inputs(), None, 3000)
        .unwrap();
    assert!(runtime.solo_active());
    runtime
        .set_live_gate(true, Duration::from_secs(4), 4000)
        .unwrap();
    runtime
        .poll(
            Duration::from_secs(5),
            Some(&snapshot(104.0, false, vec![])),
            5000,
        )
        .unwrap();
    assert!(runtime.solo_active());
    runtime.timer_pulse(Duration::from_secs(6)).unwrap();
    runtime.finish(Duration::from_secs(7), 7000).unwrap();
    assert!(
        runtime
            .take_summaries()
            .iter()
            .filter(|s| s.context.probe_game)
            .all(|s| s.metrics.pulses == 0)
    );
}
