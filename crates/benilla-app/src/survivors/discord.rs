//! The Discord status: the run's class, level, kills and battleground, shown on the player's
//! Discord profile. It goes to the local Discord client over its IPC pipe from a worker thread, so
//! a missing, closed or slow Discord never stalls a frame; with no Discord running nothing is
//! shown and nothing fails. Native only: the browser has no pipe to reach.
//!
//! Each player brings their own Discord application (its name is the "Playing …" line): its id is
//! read from `$DISCORD_APP_ID`, else from [`APP_ID_FILE`] beside the binary. With neither, the
//! status is off. The id is never compiled in.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};

use super::{Phase, Run, Selection};

/// The file beside the binary holding the Discord application id; git-ignored.
const APP_ID_FILE: &str = "discord-app-id.txt";
/// The art asset uploaded under the application's Rich Presence, under this key.
const LOGO: &str = "logo";
/// Discord accepts five updates per 20 s; one per 15 s stays inside it.
const INTERVAL: Duration = Duration::from_secs(15);
/// How far the run clock may drift from what Discord shows before it is resent: Discord counts
/// on its own from `start`, so only a pause or a new run moves it.
const CLOCK_SLACK: i64 = 3;

pub(super) fn plugin(app: &mut App) {
    let Some(app_id) = app_id() else {
        info!("discord status off: no $DISCORD_APP_ID and no {APP_ID_FILE} beside the binary");
        return;
    };
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("discord-status".into())
        .spawn(move || worker(&app_id, rx));
    if spawned.is_ok() {
        app.insert_resource(Outbox { tx, last: None })
            .add_systems(Update, publish.after(super::hud::show_panels));
    }
}

/// The Discord application id: `$DISCORD_APP_ID`, else the first line of [`APP_ID_FILE`] beside
/// the binary. Only a run of digits counts; anything else is no id.
fn app_id() -> Option<String> {
    let raw = std::env::var("DISCORD_APP_ID").ok().or_else(|| {
        let file: PathBuf = std::env::current_exe().ok()?.parent()?.join(APP_ID_FILE);
        std::fs::read_to_string(file).ok()
    })?;
    let id = raw.lines().next()?.trim();
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

/// What the profile shows: two lines and, while a run plays, its clock.
#[derive(Clone, Debug, PartialEq)]
struct Presence {
    details: String,
    state: String,
    /// Unix seconds the run started at, for Discord's own running clock.
    start: Option<i64>,
}

impl Presence {
    fn same(&self, other: &Presence) -> bool {
        self.details == other.details
            && self.state == other.state
            && match (self.start, other.start) {
                (Some(a), Some(b)) => (a - b).abs() <= CLOCK_SLACK,
                (a, b) => a == b,
            }
    }
}

#[derive(Resource)]
struct Outbox {
    tx: mpsc::Sender<Presence>,
    /// The last presence handed to the worker, so an unchanged one is not sent again.
    last: Option<Presence>,
}

fn publish(
    phase: Res<State<Phase>>,
    selection: Res<Selection>,
    run: Res<Run>,
    mut out: ResMut<Outbox>,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let presence = presence(*phase.get(), &selection, &run, now);
    if out.last.as_ref().is_some_and(|l| l.same(&presence)) {
        return;
    }
    if out.tx.send(presence.clone()).is_ok() {
        out.last = Some(presence);
    }
}

fn presence(phase: Phase, selection: &Selection, run: &Run, now: i64) -> Presence {
    let map = selection.stage_def().name;
    let hero = match race_name(run.race) {
        Some(race) => format!("{race} {}", run.class.name),
        None => run.class.name.to_string(),
    };
    let tally = match run.bosses {
        0 => format!("{} Kills", run.kills),
        1 => format!("{} Kills · 1 Boss", run.kills),
        b => format!("{} Kills · {b} Bosse", run.kills),
    };
    match phase {
        Phase::Menu => Presence {
            details: "Wählt einen Helden".into(),
            state: match selection.class_def() {
                Some(class) => format!("{} · {}", class.name, selection.map_def().name),
                None => "Im Hauptmenü".into(),
            },
            start: None,
        },
        Phase::Playing | Phase::LevelUp => Presence {
            details: format!("{hero} · Level {}", run.level),
            state: format!("{map} · {tally}"),
            start: Some(now - run.elapsed as i64),
        },
        Phase::Paused => Presence {
            details: format!("{hero} · Level {} (Pause)", run.level),
            state: format!("{map} · {tally}"),
            start: None,
        },
        Phase::GameOver => {
            let secs = run.elapsed as u32;
            let verdict = if run.won { "Sieg" } else { "Gefallen" };
            Presence {
                details: format!("{verdict} als {hero} · Level {}", run.level),
                state: format!("{map} · {:02}:{:02} · {tally}", secs / 60, secs % 60),
                start: None,
            }
        }
    }
}

/// `ChrRaces.dbc`'s playable rows, as the menu's looks carry them.
fn race_name(race: u8) -> Option<&'static str> {
    Some(match race {
        1 => "Human",
        2 => "Orc",
        3 => "Dwarf",
        4 => "Night Elf",
        5 => "Undead",
        6 => "Tauren",
        7 => "Gnome",
        8 => "Troll",
        _ => return None,
    })
}

/// The worker: holds the newest presence and sends it at most once per [`INTERVAL`], connecting
/// (and after an error, reconnecting) as it goes. Ends when the app drops its sender.
fn worker(app_id: &str, rx: mpsc::Receiver<Presence>) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut pending: Option<Presence> = None;
    let mut last_try: Option<Instant> = None;
    loop {
        let wait = match (&pending, last_try) {
            (Some(_), Some(t)) => INTERVAL.saturating_sub(t.elapsed()),
            (Some(_), None) => Duration::ZERO,
            (None, _) => Duration::from_secs(60),
        };
        match rx.recv_timeout(wait) {
            Ok(p) => pending = Some(p),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(p) = rx.try_recv() {
            pending = Some(p);
        }
        let Some(p) = &pending else { continue };
        if last_try.is_some_and(|t| t.elapsed() < INTERVAL) {
            continue;
        }
        last_try = Some(Instant::now());
        if client.is_none() {
            let mut c = DiscordIpcClient::new(app_id);
            if c.connect().is_err() {
                continue;
            }
            client = Some(c);
        }
        let c = client.as_mut().expect("connected above");
        let mut payload = activity::Activity::new()
            .details(p.details.as_str())
            .state(p.state.as_str())
            .assets(
                activity::Assets::new()
                    .large_image(LOGO)
                    .large_text("Azeroth Survivors"),
            );
        if let Some(start) = p.start {
            payload = payload.timestamps(activity::Timestamps::new().start(start));
        }
        if c.set_activity(payload).is_ok() {
            pending = None;
        } else {
            let _ = c.close();
            client = None;
        }
    }
    if let Some(mut c) = client {
        let _ = c.clear_activity();
        let _ = c.close();
    }
}
