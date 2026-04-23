use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use clap::Parser;
use regex::Regex;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

const MS_PER_SECOND: u64 = 1_000;
const MS_PER_MINUTE: u64 = 60_000;
const MS_PER_HOUR: u64 = 3_600_000;
const MS_PER_DAY: u64 = 86_400_000;

#[derive(Parser, Debug)]
#[command(
    name = "heartbeat",
    about = "Print a message repeatedly on a schedule",
    long_about = "Print a message repeatedly, either at a fixed interval after start \
or aligned to wall-clock boundaries (cron-like).\n\n\
Output goes to stdout, one line per firing. Runs forever until killed.",
    after_long_help = "\
INTERVAL SYNTAX
  Plain interval  — fires N after start, then every N:
    30m           30 minutes after start
    2h30m         2 hours 30 minutes after start
    1h15m30s      compound: 1h 15m 30s
    500ms         500 milliseconds

  Units: ms, s, m, h, d  (can be combined: 1h30m, 2d12h)

  Aligned (@)  — fires on wall-clock multiples of N from midnight:
    @15m          fires at :00, :15, :30, :45 of every hour
    @30m          fires at :00 and :30 of every hour
    @2h30m        fires at 00:00, 02:30, 05:00 … 22:30 daily
                  (last period before midnight may be shorter)
    @1d           fires at midnight every day

  Aligned with offset (@N+O)  — shifts the anchor by O:
    @1h+15m       fires at :15 past every hour (01:15, 02:15, …)
    @30m+5m       fires at :05 and :35 of every hour
    @2h30m+30m    fires at 00:30, 03:00, 05:30, …
    Offset must be strictly less than the interval.

ALIGNMENT SEMANTICS
  Anchor is local midnight. The next fire time is the smallest future
  wall-clock instant where (time_since_midnight - offset) is a
  non-negative multiple of interval. DST transitions are handled
  by chrono; edge cases at midnight may behave unexpectedly.

EXAMPLES
  heartbeat 30m \"still alive\"
      Prints \"still alive\" every 30 minutes starting 30m from now.

  heartbeat -p @15m \"ping\"
      Prints a timestamped \"ping\" at :00, :15, :30, :45 of every hour.

  heartbeat @1h+15m \"hourly report\"
      Prints \"hourly report\" at 15 minutes past every hour.

  heartbeat @2h30m \"check in\"
      Prints \"check in\" at 00:00, 02:30, 05:00 … 22:30 each day.

  heartbeat 500ms \"fast tick\"
      Prints \"fast tick\" every 500 milliseconds.

FOR AI AGENTS
  Use this tool to emit a periodic stdout signal during long-running
  tasks so that a supervising process knows the agent is still alive.
  Recommended pattern:

    heartbeat @5m \"agent heartbeat\" &
    HB_PID=$!
    # ... do long work ...
    kill $HB_PID

  The -p/--print-datetime flag adds an ISO-8601 timestamp prefix to
  each line, useful for log correlation:

    2026-04-22 14:30:00.123 agent heartbeat"
)]
struct Args {
    #[arg(
        short,
        long,
        default_value = "false",
        help = "Prefix each message with a timestamp (YYYY-MM-DD HH:MM:SS.mmm)"
    )]
    print_datetime: bool,

    #[arg(
        long,
        default_value = "true",
        help = "Prefix each output line with [HH:MM] showing current time"
    )]
    time_prefix: bool,

    #[arg(
        long,
        default_value = "false",
        help = "Use UTC timezone for time prefix instead of local"
    )]
    time_prefix_utc: bool,

    #[arg(
        help = "How often to fire. Plain (30m, 2h30m) or aligned (@15m, @1h+15m). \
See --help for full syntax."
    )]
    interval: String,

    #[arg(
        short = 'n',
        long = "nb-iters",
        help = "Number of times to fire before exiting (default: run forever)"
    )]
    nb_iters: Option<u64>,

    #[arg(help = "Message to print on each firing")]
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Schedule {
    interval: Duration,
    offset: Duration,
    aligned: bool,
}

fn duration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d+(?:\.\d+)?)(ms|[dhms])").unwrap())
}

fn parse_compound(s: &str) -> Result<Duration, String> {
    if s.is_empty() {
        return Err("empty duration".to_string());
    }
    let mut total_ms: f64 = 0.0;
    let mut cursor = 0;
    for caps in duration_re().captures_iter(s) {
        let m = caps.get(0).unwrap();
        if m.start() != cursor {
            return Err(format!("invalid duration: {}", s));
        }
        cursor = m.end();
        let value: f64 = caps[1].parse().map_err(|_| "invalid number".to_string())?;
        let unit = &caps[2];
        let ms = match unit {
            "ms" => value,
            "s" => value * MS_PER_SECOND as f64,
            "m" => value * MS_PER_MINUTE as f64,
            "h" => value * MS_PER_HOUR as f64,
            "d" => value * MS_PER_DAY as f64,
            _ => return Err(format!("unknown unit: {}", unit)),
        };
        total_ms += ms;
    }
    if cursor != s.len() {
        return Err(format!("invalid duration: {}", s));
    }
    Ok(Duration::from_millis(total_ms as u64))
}

fn parse_schedule(s: &str) -> Result<Schedule, String> {
    let (aligned, rest) = match s.strip_prefix('@') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (interval_str, offset_str) = match rest.split_once('+') {
        Some((i, o)) => (i, Some(o)),
        None => (rest, None),
    };
    if offset_str.is_some() && !aligned {
        return Err("offset (+...) requires aligned mode (@)".to_string());
    }
    let interval = parse_compound(interval_str)?;
    if interval.is_zero() {
        return Err("interval must be non-zero".to_string());
    }
    let offset = match offset_str {
        Some(o) => parse_compound(o)?,
        None => Duration::ZERO,
    };
    if aligned && offset >= interval {
        return Err("offset must be strictly less than interval".to_string());
    }
    Ok(Schedule {
        interval,
        offset,
        aligned,
    })
}

fn compute_next_fire(now: DateTime<Local>, schedule: &Schedule) -> DateTime<Local> {
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .earliest()
        .expect("local midnight does not exist (DST edge)");

    let interval_ms = i64::try_from(schedule.interval.as_millis())
        .expect("interval too large for scheduling arithmetic");
    let offset_ms = i64::try_from(schedule.offset.as_millis())
        .expect("offset too large for scheduling arithmetic");
    let since_midnight_ms = (now - midnight).num_milliseconds();

    let adjusted = since_midnight_ms - offset_ms;
    let k = if adjusted < 0 {
        0
    } else {
        adjusted / interval_ms + 1
    };
    let target_ms = k * interval_ms + offset_ms;

    let day_ms = MS_PER_DAY as i64;
    if target_ms >= day_ms {
        let next_midnight = midnight + ChronoDuration::days(1);
        next_midnight + ChronoDuration::milliseconds(offset_ms)
    } else {
        midnight + ChronoDuration::milliseconds(target_ms)
    }
}

fn format_duration(d: Duration) -> String {
    let mut ms = d.as_millis();
    let days = ms / MS_PER_DAY as u128;
    ms %= MS_PER_DAY as u128;
    let hours = ms / MS_PER_HOUR as u128;
    ms %= MS_PER_HOUR as u128;
    let mins = ms / MS_PER_MINUTE as u128;
    ms %= MS_PER_MINUTE as u128;
    let secs = ms / MS_PER_SECOND as u128;
    ms %= MS_PER_SECOND as u128;

    let mut out = String::new();
    if days > 0 {
        out += &format!("{}d", days);
    }
    if hours > 0 {
        out += &format!("{}h", hours);
    }
    if mins > 0 {
        out += &format!("{}m", mins);
    }
    if secs > 0 {
        out += &format!("{}s", secs);
    }
    if ms > 0 {
        out += &format!("{}ms", ms);
    }
    if out.is_empty() {
        out.push_str("0s");
    }
    out
}

fn main() {
    let args = Args::parse();
    let schedule = parse_schedule(&args.interval).expect("failed to parse interval");

    if schedule.aligned {
        let offset_suffix = if schedule.offset.is_zero() {
            String::new()
        } else {
            format!(" (offset {})", format_duration(schedule.offset))
        };
        println!(
            "heartbeat will repeat \"{}\" every {} aligned to midnight{}",
            args.message,
            format_duration(schedule.interval),
            offset_suffix
        );
    } else {
        let minutes = schedule.interval.as_secs_f64() / 60.0;
        println!(
            "heartbeat is set up to repeat \"{}\" every {:.2} minutes",
            args.message, minutes
        );
    }

    let mut iters: u64 = 0;
    loop {
        if let Some(max) = args.nb_iters {
            if iters >= max {
                break;
            }
        }
        let now = Local::now();
        let next = if schedule.aligned {
            compute_next_fire(now, &schedule)
        } else {
            now + ChronoDuration::from_std(schedule.interval)
                .expect("interval too large")
        };
        let wait = (next - now).to_std().unwrap_or(Duration::ZERO);
        thread::sleep(wait);
        let prefix = if args.time_prefix {
            let time_str = if args.time_prefix_utc {
                next.with_timezone(&Utc).format("%H:%M").to_string()
            } else {
                next.format("%H:%M").to_string()
            };
            format!("[{}] ", time_str)
        } else {
            String::new()
        };
        let countdown = args.nb_iters.map(|max| {
            let remaining = max - iters;
            format!("[{}/{}] ", remaining, max)
        }).unwrap_or_default();
        if args.print_datetime {
            let datetime_str = if args.time_prefix_utc {
                next.with_timezone(&Utc).format("%Y-%m-%d %H:%M:%S%.3f").to_string()
            } else {
                next.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
            };
            println!("{}{}{} {}", prefix, countdown, datetime_str, args.message);
        } else {
            println!("{}{}{}", prefix, countdown, args.message);
        }
        iters += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn d(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn parse_single_token() {
        assert_eq!(parse_compound("30m").unwrap(), d(30 * 60_000));
        assert_eq!(parse_compound("500ms").unwrap(), d(500));
        assert_eq!(parse_compound("1.5s").unwrap(), d(1500));
    }

    #[test]
    fn parse_compound_tokens() {
        assert_eq!(parse_compound("2h30m").unwrap(), d(150 * 60_000));
        assert_eq!(
            parse_compound("1h15m30s").unwrap(),
            d(60 * 60_000 + 15 * 60_000 + 30_000)
        );
        assert_eq!(parse_compound("1d2h").unwrap(), d(26 * 3_600_000));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_compound("").is_err());
        assert!(parse_compound("30").is_err());
        assert!(parse_compound("m30").is_err());
        assert!(parse_compound("30m junk").is_err());
        assert!(parse_compound("30x").is_err());
    }

    #[test]
    fn schedule_aligned_no_offset() {
        let s = parse_schedule("@30m").unwrap();
        assert!(s.aligned);
        assert_eq!(s.interval, d(30 * 60_000));
        assert_eq!(s.offset, Duration::ZERO);
    }

    #[test]
    fn schedule_aligned_with_offset() {
        let s = parse_schedule("@1h+15m").unwrap();
        assert!(s.aligned);
        assert_eq!(s.interval, d(3_600_000));
        assert_eq!(s.offset, d(15 * 60_000));
    }

    #[test]
    fn schedule_unaligned() {
        let s = parse_schedule("30m").unwrap();
        assert!(!s.aligned);
        assert_eq!(s.interval, d(30 * 60_000));
        assert_eq!(s.offset, Duration::ZERO);
    }

    #[test]
    fn schedule_rejects_bad_forms() {
        assert!(parse_schedule("30m+5m").is_err(), "offset without @");
        assert!(parse_schedule("@30m+45m").is_err(), "offset >= interval");
        assert!(parse_schedule("@30m+30m").is_err(), "offset == interval");
        assert!(parse_schedule("@0s").is_err(), "zero interval");
    }

    fn at(h: u32, m: u32, s: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2024, 6, 15, h, m, s).unwrap()
    }

    #[test]
    fn next_fire_quarter_hour() {
        let s = parse_schedule("@15m").unwrap();
        assert_eq!(compute_next_fire(at(10, 7, 0), &s), at(10, 15, 0));
        assert_eq!(compute_next_fire(at(10, 15, 0), &s), at(10, 30, 0));
    }

    #[test]
    fn next_fire_hour_with_offset() {
        let s = parse_schedule("@1h+15m").unwrap();
        assert_eq!(compute_next_fire(at(10, 7, 0), &s), at(10, 15, 0));
        assert_eq!(compute_next_fire(at(10, 20, 0), &s), at(11, 15, 0));
    }

    #[test]
    fn next_fire_rolls_past_midnight() {
        let s = parse_schedule("@2h30m").unwrap();
        let now = at(22, 45, 0);
        let next = compute_next_fire(now, &s);
        let expected = Local.with_ymd_and_hms(2024, 6, 16, 0, 0, 0).unwrap();
        assert_eq!(next, expected);
    }

    #[test]
    fn next_fire_rolls_past_midnight_with_offset() {
        let s = parse_schedule("@2h30m+30m").unwrap();
        let now = at(23, 30, 0);
        let next = compute_next_fire(now, &s);
        let expected = Local.with_ymd_and_hms(2024, 6, 16, 0, 30, 0).unwrap();
        assert_eq!(next, expected);
    }

    #[test]
    fn format_duration_components() {
        assert_eq!(format_duration(d(150 * 60_000)), "2h30m");
        assert_eq!(format_duration(d(15 * 60_000)), "15m");
        assert_eq!(format_duration(d(500)), "500ms");
        assert_eq!(format_duration(Duration::ZERO), "0s");
    }

    #[test]
    fn time_prefix_format_local() {
        let local_time = Local::now();
        let formatted = local_time.format("%H:%M").to_string();
        assert_eq!(formatted.len(), 5);
        assert!(formatted.chars().nth(2) == Some(':'));
    }

    #[test]
    fn time_prefix_format_utc() {
        let utc_time = Utc::now();
        let formatted = utc_time.format("%H:%M").to_string();
        assert_eq!(formatted.len(), 5);
        assert!(formatted.chars().nth(2) == Some(':'));
    }
}
