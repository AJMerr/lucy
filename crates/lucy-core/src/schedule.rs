//! Reminders and scheduled tasks: when they run (`When`, in local time), the clock the core
//! runs them by (`Clock`, replaced in tests), and how the core tells you when no app is there
//! to (`Notifier`).

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime,
    TimeZone, Weekday,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

/// When a reminder or task runs.
#[derive(Debug, Clone, PartialEq)]
pub enum When {
    /// Once, at this local time.
    At(NaiveDateTime),
    Daily(NaiveTime),
    /// Monday to Friday.
    Weekdays(NaiveTime),
    Weekly(Vec<Weekday>, NaiveTime),
    /// On this day of each month (the last day in shorter months).
    Monthly(u32, NaiveTime),
    /// Every this many minutes.
    Every(u32),
}

impl When {
    /// From what `schedule` is given: `{"at": "2026-11-12T17:00"}`, `{"daily": "08:00"}`,
    /// `{"weekdays": "08:00"}`, `{"weekly": {"days": ["mon", "fri"], "time": "09:30"}}`,
    /// `{"monthly": {"day": 1, "time": "09:00"}}` or `{"every_minutes": 30}`.
    pub fn parse(spec: &Value) -> Result<Self, String> {
        let time = |value: &Value| -> Result<NaiveTime, String> {
            let text = value.as_str().ok_or("a time like \"08:00\"")?;
            NaiveTime::parse_from_str(text, "%H:%M")
                .or_else(|_| NaiveTime::parse_from_str(text, "%H:%M:%S"))
                .map_err(|_| format!("\"{text}\" isn't a time like \"08:00\""))
        };
        let object = spec
            .as_object()
            .filter(|o| o.len() == 1)
            .ok_or_else(|| format!("`when` needs exactly one of at, daily, weekdays, weekly, monthly or every_minutes, not {spec}"))?;
        let (key, value) = object.iter().next().expect("one entry");
        match key.as_str() {
            "at" => {
                let text = value
                    .as_str()
                    .ok_or("`at` takes a time like \"2026-11-12T17:00\"")?;
                NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M")
                    .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S"))
                    .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M"))
                    .map(When::At)
                    .map_err(|_| format!("\"{text}\" isn't a local time like \"2026-11-12T17:00\""))
            }
            "daily" => Ok(When::Daily(time(value)?)),
            "weekdays" => Ok(When::Weekdays(time(value)?)),
            "weekly" => {
                let days = value["days"]
                    .as_array()
                    .ok_or("`weekly` takes {\"days\": [\"mon\", …], \"time\": \"09:30\"}")?
                    .iter()
                    .map(|d| {
                        d.as_str()
                            .and_then(|d| d.parse::<Weekday>().ok())
                            .ok_or_else(|| format!("{d} isn't a day like \"mon\""))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if days.is_empty() {
                    return Err("`weekly` needs at least one day".into());
                }
                Ok(When::Weekly(days, time(&value["time"])?))
            }
            "monthly" => {
                let day = value["day"]
                    .as_u64()
                    .filter(|d| (1..=31).contains(d))
                    .ok_or("`monthly` takes {\"day\": 1-31, \"time\": \"09:00\"}")?;
                Ok(When::Monthly(day as u32, time(&value["time"])?))
            }
            "every_minutes" => {
                let minutes = value
                    .as_u64()
                    .filter(|m| *m >= 1)
                    .ok_or("`every_minutes` takes a number of minutes")?;
                Ok(When::Every(minutes.min(u64::from(u32::MAX)) as u32))
            }
            other => Err(format!(
                "`when` can't be \"{other}\"; use at, daily, weekdays, weekly, monthly or every_minutes"
            )),
        }
    }

    /// As `schedule` takes it.
    pub fn spec(&self) -> Value {
        let hm = |t: &NaiveTime| t.format("%H:%M").to_string();
        match self {
            When::At(at) => json!({"at": at.format("%Y-%m-%dT%H:%M").to_string()}),
            When::Daily(t) => json!({"daily": hm(t)}),
            When::Weekdays(t) => json!({"weekdays": hm(t)}),
            When::Weekly(days, t) => json!({"weekly": {
                "days": days.iter().map(|d| d.to_string().to_lowercase()).collect::<Vec<_>>(),
                "time": hm(t),
            }}),
            When::Monthly(day, t) => json!({"monthly": {"day": day, "time": hm(t)}}),
            When::Every(minutes) => json!({"every_minutes": minutes}),
        }
    }

    /// In words: "every day at 08:00", "weekdays at 08:00", "once on Thu 12 Nov at 17:00".
    pub fn describe(&self) -> String {
        let hm = |t: &NaiveTime| t.format("%H:%M").to_string();
        match self {
            When::At(at) => format!("once on {}", at.format("%a %-d %b %Y at %H:%M")),
            When::Daily(t) => format!("every day at {}", hm(t)),
            When::Weekdays(t) => format!("weekdays at {}", hm(t)),
            When::Weekly(days, t) => {
                let names: Vec<String> =
                    days.iter().map(|d| format!("{}s", day_name(*d))).collect();
                format!("{} at {}", join_words(&names), hm(t))
            }
            When::Monthly(day, t) => format!("on the {} of each month at {}", ordinal(*day), hm(t)),
            When::Every(minutes) if minutes % 60 == 0 && *minutes >= 60 => match minutes / 60 {
                1 => "every hour".into(),
                hours => format!("every {hours} hours"),
            },
            When::Every(1) => "every minute".into(),
            When::Every(minutes) => format!("every {minutes} minutes"),
        }
    }

    pub fn is_once(&self) -> bool {
        matches!(self, When::At(_))
    }

    /// The first time strictly after `after`, if there is one.
    pub fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            When::At(at) => local(*at).filter(|t| *t > after),
            When::Every(minutes) => Some(after + Duration::minutes(i64::from(*minutes))),
            When::Daily(t) => next_day(after, *t, |_| true),
            When::Weekdays(t) => next_day(after, *t, |date| {
                !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
            }),
            When::Weekly(days, t) => next_day(after, *t, |date| days.contains(&date.weekday())),
            When::Monthly(day, t) => next_day(after, *t, |date| {
                let last = last_day_of_month(date);
                date.day() == (*day).min(last)
            }),
        }
    }
}

/// The first day from `after`'s date on that `fits`, at `time`, after `after`.
fn next_day(
    after: DateTime<Local>,
    time: NaiveTime,
    fits: impl Fn(NaiveDate) -> bool,
) -> Option<DateTime<Local>> {
    let mut date = after.date_naive();
    for _ in 0..800 {
        if fits(date)
            && let Some(at) = local(date.and_time(time))
            && at > after
        {
            return Some(at);
        }
        date = date.succ_opt()?;
    }
    None
}

/// A local time; across a DST jump forward, the first valid moment after it.
fn local(at: NaiveDateTime) -> Option<DateTime<Local>> {
    match Local.from_local_datetime(&at) {
        LocalResult::Single(t) => Some(t),
        LocalResult::Ambiguous(first, _) => Some(first),
        LocalResult::None => Local
            .from_local_datetime(&(at + Duration::hours(1)))
            .earliest(),
    }
}

fn last_day_of_month(date: NaiveDate) -> u32 {
    let (y, m) = (date.year(), date.month());
    let first_next = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, 1)
    };
    first_next
        .and_then(|d| d.pred_opt())
        .map_or(28, |d| d.day())
}

fn day_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

fn join_words(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

// --- The clock -----------------------------------------------------------------------------------

/// What the core reads the time from and waits on.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Local>;
    /// Resolves at `when` (or straight away if it's past).
    fn sleep_until(&self, when: DateTime<Local>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Local> {
        Local::now()
    }

    fn sleep_until(&self, when: DateTime<Local>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // Wake at least every minute, so a suspended laptop or a changed clock catches up.
            loop {
                let left = (when - Local::now()).to_std().unwrap_or_default();
                if left.is_zero() {
                    return;
                }
                tokio::time::sleep(left.min(std::time::Duration::from_secs(60))).await;
            }
        })
    }
}

/// A clock tests move by hand.
pub struct FakeClock {
    now: Mutex<DateTime<Local>>,
    moved: Notify,
}

impl FakeClock {
    pub fn new(now: DateTime<Local>) -> Self {
        Self {
            now: Mutex::new(now),
            moved: Notify::new(),
        }
    }

    pub fn set(&self, now: DateTime<Local>) {
        *self.now.lock().unwrap() = now;
        self.moved.notify_waiters();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Local> {
        *self.now.lock().unwrap()
    }

    fn sleep_until(&self, when: DateTime<Local>) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            loop {
                let moved = self.moved.notified();
                if self.now() >= when {
                    return;
                }
                moved.await;
            }
        })
    }
}

// --- Notifications -------------------------------------------------------------------------------

/// How the core tells you something when no app is showing notifications.
pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

/// A notification body shown as written: Linux notification servers read a little HTML
/// (links, images), which text from a model or a task shouldn't be able to use.
fn plain_body(body: &str) -> String {
    if cfg!(target_os = "linux") {
        body.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    } else {
        body.to_owned()
    }
}

/// A desktop notification (D-Bus on Linux, Notification Center on macOS, toasts on Windows).
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&self, title: &str, body: &str) {
        let shown = notify_rust::Notification::new()
            .appname("Lucy")
            .summary(title)
            .body(&plain_body(body))
            .show();
        if let Err(err) = shown {
            eprintln!("couldn't show a notification: {err}");
        }
    }
}
