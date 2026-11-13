//! Reminders and scheduled tasks: the loop that runs them on the core's clock, the Tasks chat
//! their results go to, notifications, the `schedule` tools' book, and `schedules.*`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Local, TimeZone};
use lucy_protocol::AgentLevel;
use lucy_protocol::rpc::{Event, ScheduleInfo, ScheduleUpdate, SchedulesList};
use serde_json::Value;
use ulid::Ulid;

use super::{Core, RpcError, TaskRun, TurnRequest, app, start_turn, working_folder};
use crate::schedule::When;
use crate::sessions::{NewMessage, Origin, Schedule, SessionId, StoreError};
use crate::tools::ScheduleBook;

const REMINDER: &str = "reminder";
const TASK: &str = "task";
/// The Tasks chat's title.
const TASKS: &str = "Tasks";
/// Longest notification text; the whole result is in the Tasks chat.
const NOTIFICATION_CHARS: usize = 300;

/// Run due reminders and tasks until the core stops.
pub(super) async fn run(core: Arc<Core>) {
    loop {
        // Before reading the schedules, so a change made meanwhile still wakes us.
        let changed = core.schedules_changed.notified();
        let now = core.clock.now();
        let schedules = core.store.lock().unwrap().schedules().unwrap_or_default();
        for schedule in schedules
            .iter()
            .filter(|s| !s.paused && s.next_run.is_some_and(|t| t <= now.timestamp_millis()))
        {
            core.fire(schedule.clone(), now);
        }
        core.count_schedules();
        let next = core
            .store
            .lock()
            .unwrap()
            .schedules()
            .unwrap_or_default()
            .iter()
            .filter(|s| !s.paused)
            .filter_map(|s| s.next_run)
            .min()
            .and_then(|ms| Local.timestamp_millis_opt(ms).single());
        let sleep = async {
            match next {
                Some(when) => core.clock.sleep_until(when).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = core.shutdown.cancelled() => return,
            _ = changed => {}
            _ = sleep => {}
        }
    }
}

impl Core {
    /// Tell the user something now: every client sees the event; if none of them shows
    /// notifications, the core sends one itself.
    pub(super) fn notify(&self, session_id: &str, title: &str, body: &str) {
        let _ = self.events.send(Event::Notification {
            session_id: session_id.to_owned(),
            title: title.to_owned(),
            body: body.to_owned(),
        });
        if self.notifying.load(Ordering::SeqCst) == 0 {
            self.notifier.notify(title, body);
        }
    }

    /// The chat reminders and task results go to, created the first time.
    pub(super) fn tasks_session(&self) -> Result<SessionId, StoreError> {
        let file = self
            .options
            .db_path
            .parent()
            .map(|dir| dir.join("tasks-session"));
        let store = self.store.lock().unwrap();
        if let Some(saved) = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok())
            && let Ok(id) = store.resolve(saved.trim())
        {
            return Ok(id);
        }
        let id = store.create()?;
        store.rename(&id, TASKS)?;
        if let Some(file) = file {
            let _ = std::fs::write(file, id.to_string());
        }
        Ok(id)
    }

    /// Keep an on-demand core running while anything is scheduled.
    pub(super) fn count_schedules(&self) {
        let active = self
            .store
            .lock()
            .unwrap()
            .schedules()
            .map(|all| {
                all.iter()
                    .filter(|s| !s.paused && s.next_run.is_some())
                    .count()
            })
            .unwrap_or(0);
        self.activity.set(&self.activity.schedules, active);
    }

    /// The core runs at login (as a service), so schedules run even if nothing starts it.
    pub(super) fn always_on(&self) -> bool {
        self.options.idle_timeout.is_none()
    }

    fn fire(self: &Arc<Self>, mut schedule: Schedule, now: DateTime<Local>) {
        let when = When::parse(&serde_json::from_str(&schedule.spec).unwrap_or(Value::Null));
        let fired = match self.tasks_session() {
            Err(err) => Err(err.to_string()),
            Ok(session) if schedule.kind == REMINDER => self.remind(&session, &schedule),
            Ok(session) => start_turn(
                self,
                TurnRequest {
                    session_id: session,
                    text: schedule.what.clone(),
                    images: Vec::new(),
                    cwd: working_folder(None),
                    remote: false,
                    started_by: None,
                    incognito: false,
                    task: Some(TaskRun {
                        title: schedule.title.clone(),
                        profile: schedule.profile.clone(),
                        level: self.task_level(&schedule),
                    }),
                },
            )
            .map(|_| ())
            .map_err(|err| err.message),
        };
        match (fired, &when) {
            // Busy (another task is replying in the Tasks chat): try again in a minute.
            (Err(err), _) if err.contains("still replying") => {
                schedule.next_run = Some((now + chrono::Duration::minutes(1)).timestamp_millis());
            }
            (result, when) => {
                if let Err(err) = result {
                    eprintln!("schedule `{}` failed: {err}", schedule.title);
                    if let Ok(session) = self.tasks_session() {
                        self.notify(
                            &session.to_string(),
                            &schedule.title,
                            &format!("Couldn't run: {err}"),
                        );
                    }
                }
                schedule.last_run = Some(now.timestamp_millis());
                schedule.next_run = match when {
                    Ok(when) => when.next_after(now).map(|t| t.timestamp_millis()),
                    Err(_) => None,
                };
            }
        }
        let store = self.store.lock().unwrap();
        let finished = schedule.next_run.is_none();
        let _ = if finished {
            store.delete_schedule(&schedule.id)
        } else {
            store.save_schedule(&schedule)
        };
    }

    /// The level a task runs at: its own, but one created or changed from another machine
    /// never above `[remote] max_level` as it is now.
    fn task_level(&self, schedule: &Schedule) -> AgentLevel {
        let level = AgentLevel::try_from(schedule.level).unwrap_or_default();
        match (schedule.remote, self.config()) {
            (false, _) => level,
            (true, Ok(config)) => level.min(config.remote.max_level),
            (true, Err(_)) => AgentLevel::Supervised,
        }
    }

    fn remind(&self, session: &SessionId, schedule: &Schedule) -> Result<(), String> {
        let origin = Origin {
            profile: REMINDER.into(),
            model: REMINDER.into(),
        };
        self.store
            .lock()
            .unwrap()
            .append(
                session,
                NewMessage::assistant(format!("⏰ {}", schedule.what), origin),
            )
            .map_err(|err| err.to_string())?;
        let title = if schedule.title == schedule.what {
            "Reminder".to_owned()
        } else {
            format!("Reminder: {}", schedule.title)
        };
        self.notify(&session.to_string(), &title, &schedule.what);
        Ok(())
    }
}

/// What a finished task tells you: its last words, or why it failed.
pub(super) fn task_summary(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= NOTIFICATION_CHARS {
        return text.to_owned();
    }
    let cut: String = text.chars().take(NOTIFICATION_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

// --- The `schedule` tools ------------------------------------------------------------------------

/// The core's schedules, for one chat's tools: new tasks use that chat's model.
pub(super) struct Book {
    pub core: Arc<Core>,
    pub profile: String,
    /// The chat is driven from another machine: its tasks are marked as such (and capped at
    /// `max_level`), and it may only cancel those.
    pub remote: bool,
    pub max_level: AgentLevel,
}

impl Book {
    /// A schedule from `schedule`'s arguments, as this chat would make it.
    fn parse(&self, arguments: &Value) -> Result<(Schedule, When), String> {
        let (mut schedule, when) = parse(arguments, &self.profile)?;
        if self.remote {
            schedule.remote = true;
            schedule.level = schedule.level.min(u8::from(self.max_level));
        }
        Ok((schedule, when))
    }
}

/// A schedule from `schedule`'s arguments.
fn parse(arguments: &Value, profile: &str) -> Result<(Schedule, When), String> {
    let kind = match arguments["kind"].as_str() {
        Some(REMINDER) => REMINDER,
        Some(TASK) => TASK,
        _ => return Err("`kind` must be \"reminder\" or \"task\"".into()),
    };
    let what = arguments["what"]
        .as_str()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .ok_or("give `what` to remind or do")?;
    let when = When::parse(&arguments["when"])?;
    let level = match arguments["level"].as_u64() {
        None => 1,
        Some(level @ 0..=3) => level as u8,
        Some(_) => return Err("`level` must be 0-3".into()),
    };
    let title = arguments["title"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map_or_else(|| short(what), str::to_owned);
    Ok((
        Schedule {
            id: Ulid::generate().to_string(),
            kind: kind.into(),
            title,
            what: what.to_owned(),
            spec: when.spec().to_string(),
            level,
            profile: profile.to_owned(),
            paused: false,
            next_run: None,
            last_run: None,
            remote: false,
        },
        when,
    ))
}

fn short(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().take(6).collect();
    let mut title = words.join(" ");
    if text.split_whitespace().count() > 6 {
        title.push('…');
    }
    title
}

/// "Reminder · once on Tue 10 Nov 2026 at 17:00\nCall Mom" and the like.
fn card(schedule: &Schedule, when: &When) -> String {
    if schedule.kind == REMINDER {
        format!("Reminder · {}\n{}", when.describe(), schedule.what)
    } else {
        format!(
            "Task “{}” · {} · level {} · profile {}\n{}",
            schedule.title,
            when.describe(),
            schedule.level,
            schedule.profile,
            schedule.what
        )
    }
}

impl ScheduleBook for Book {
    fn describe(&self, arguments: &Value) -> Result<(String, String), String> {
        let (schedule, when) = self.parse(arguments)?;
        if when.next_after(self.core.clock.now()).is_none() {
            return Err(format!("{} is in the past", when.describe()));
        }
        let reason = if schedule.kind == REMINDER {
            "schedules a reminder".to_owned()
        } else {
            format!(
                "schedules a task that runs at level {} while you're away",
                schedule.level
            )
        };
        Ok((reason, card(&schedule, &when)))
    }

    fn create(&self, arguments: &Value) -> Result<String, String> {
        let (mut schedule, when) = self.parse(arguments)?;
        let next = when
            .next_after(self.core.clock.now())
            .ok_or_else(|| format!("{} is in the past", when.describe()))?;
        schedule.next_run = Some(next.timestamp_millis());
        self.core
            .store
            .lock()
            .unwrap()
            .add_schedule(&schedule)
            .map_err(|err| err.to_string())?;
        self.core.schedules_changed.notify_waiters();
        self.core.count_schedules();
        let mut text = format!(
            "Scheduled {} “{}”, {}. Next: {}.",
            schedule.kind,
            schedule.title,
            when.describe(),
            next.format("%a %-d %b at %H:%M")
        );
        if schedule.kind == TASK {
            text += " Results will be in the Tasks chat, and the user is notified.";
        }
        if !self.core.always_on() {
            text += " Lucy's core isn't set to start at login, so this only runs while it's \
                     running (the computer is on and Lucy was started). Offer to make it start \
                     at login: `lucy service install` (or Settings → Service & startup).";
        }
        Ok(text)
    }

    fn list(&self) -> String {
        let all = self
            .core
            .store
            .lock()
            .unwrap()
            .schedules()
            .unwrap_or_default();
        if all.is_empty() {
            return "Nothing is scheduled.".into();
        }
        all.iter()
            .map(|s| {
                let info = info(s);
                let next = info
                    .next_run_ms
                    .and_then(|ms| Local.timestamp_millis_opt(ms).single());
                format!(
                    "- {} “{}” ({}), {}{}{}",
                    s.kind,
                    s.title,
                    s.what,
                    info.when,
                    next.map_or_else(String::new, |t| format!(
                        ", next {}",
                        t.format("%a %-d %b %H:%M")
                    )),
                    if s.paused { ", paused" } else { "" },
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn cancel(&self, which: &str) -> Result<String, String> {
        let which = which.trim().to_lowercase();
        let store = self.core.store.lock().unwrap();
        let mut all = store.schedules().map_err(|err| err.to_string())?;
        // From another machine, only what was scheduled from one.
        if self.remote {
            all.retain(|s| s.remote);
        }
        let matches: Vec<&Schedule> = all
            .iter()
            .filter(|s| s.id.to_lowercase() == which || s.title.to_lowercase() == which)
            .chain(
                all.iter()
                    .filter(|s| s.title.to_lowercase().contains(&which)),
            )
            .collect();
        let target = match matches.first() {
            Some(first) if matches.iter().all(|s| s.id == first.id) => *first,
            Some(_) => {
                return Err(format!(
                    "\"{which}\" matches several; name one exactly: {}",
                    all.iter()
                        .filter(|s| s.title.to_lowercase().contains(&which))
                        .map(|s| format!("“{}”", s.title))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            None => return Err(format!("nothing scheduled matches \"{which}\"")),
        };
        store
            .delete_schedule(&target.id)
            .map_err(|err| err.to_string())?;
        drop(store);
        self.core.schedules_changed.notify_waiters();
        self.core.count_schedules();
        Ok(format!("Cancelled {} “{}”.", target.kind, target.title))
    }
}

// --- schedules.* ---------------------------------------------------------------------------------

pub(super) fn info(s: &Schedule) -> ScheduleInfo {
    let spec: Value = serde_json::from_str(&s.spec).unwrap_or(Value::Null);
    ScheduleInfo {
        id: s.id.clone(),
        kind: s.kind.clone(),
        title: s.title.clone(),
        what: s.what.clone(),
        when: When::parse(&spec).map_or_else(|_| "never".into(), |w| w.describe()),
        when_spec: spec,
        level: s.level,
        profile: s.profile.clone(),
        paused: s.paused,
        next_run_ms: s.next_run,
        last_run_ms: s.last_run,
    }
}

pub(super) fn list(core: &Core) -> Result<SchedulesList, RpcError> {
    let all = core.store.lock().unwrap().schedules().map_err(app)?;
    Ok(SchedulesList {
        schedules: all.iter().map(info).collect(),
        always_on: core.always_on(),
    })
}

/// `schedules.update`. A change from another machine marks the task as remote, so it runs at
/// most at `[remote] max_level`.
pub(super) fn update(
    core: &Core,
    change: ScheduleUpdate,
    remote: bool,
) -> Result<ScheduleInfo, RpcError> {
    let mut schedule = core
        .store
        .lock()
        .unwrap()
        .schedule(&change.id)
        .map_err(app)?;
    if remote {
        schedule.remote = true;
    }
    if let Some(title) = change.title.filter(|t| !t.trim().is_empty()) {
        schedule.title = title.trim().to_owned();
    }
    if let Some(what) = change.what.filter(|w| !w.trim().is_empty()) {
        schedule.what = what.trim().to_owned();
    }
    if let Some(level) = change.level {
        if level > 3 {
            return Err(RpcError::app("the level must be 0-3"));
        }
        schedule.level = level;
    }
    if let Some(profile) = change.profile {
        if !core.config()?.profiles.contains_key(&profile) {
            return Err(RpcError::app(format!("no profile `{profile}`")));
        }
        schedule.profile = profile;
    }
    let now = core.clock.now();
    if let Some(spec) = change.when {
        let when = When::parse(&spec).map_err(RpcError::app)?;
        schedule.spec = when.spec().to_string();
        schedule.next_run = Some(
            when.next_after(now)
                .ok_or_else(|| RpcError::app(format!("{} is in the past", when.describe())))?
                .timestamp_millis(),
        );
    }
    if let Some(paused) = change.paused {
        // Unpausing starts from now: runs missed while paused don't all come at once.
        if schedule.paused && !paused {
            let when = When::parse(&serde_json::from_str(&schedule.spec).unwrap_or(Value::Null))
                .map_err(RpcError::app)?;
            schedule.next_run = when.next_after(now).map(|t| t.timestamp_millis());
        }
        schedule.paused = paused;
    }
    core.store
        .lock()
        .unwrap()
        .save_schedule(&schedule)
        .map_err(app)?;
    core.schedules_changed.notify_waiters();
    core.count_schedules();
    Ok(info(&schedule))
}

pub(super) fn delete(core: &Core, id: &str) -> Result<(), RpcError> {
    core.store
        .lock()
        .unwrap()
        .delete_schedule(id)
        .map_err(app)?;
    core.schedules_changed.notify_waiters();
    core.count_schedules();
    Ok(())
}
