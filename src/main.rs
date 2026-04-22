use chrono::Local;
use clap::Parser;
use regex::Regex;
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "heartbeat", about = "Repeat a message at a given interval")]
struct Args {
    #[arg(short, long, default_value = "false")]
    print_datetime: bool,

    interval: String,
    message: String,
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    let re = Regex::new(r"^(\d+(?:\.\d+)?)([dhms]|ms)$").unwrap();

    if let Some(caps) = re.captures(s) {
        let value: f64 = caps.get(1).unwrap().as_str().parse().map_err(|_| "Invalid number")?;
        let unit = caps.get(2).unwrap().as_str();

        let ms = match unit {
            "ms" => value,
            "s" => value * 1000.0,
            "m" => value * 60.0 * 1000.0,
            "h" => value * 3600.0 * 1000.0,
            "d" => value * 86400.0 * 1000.0,
            _ => return Err("Unknown unit".to_string()),
        };

        Ok(Duration::from_millis(ms as u64))
    } else {
        Err("Invalid interval format".to_string())
    }
}

fn main() {
    let args = Args::parse();

    let duration = parse_duration(&args.interval).expect("Failed to parse interval");
    let total_secs = duration.as_secs_f64();
    let minutes = total_secs / 60.0;

    println!(
        "heartbeat is set up to repeat \"{}\" every {:.2} minutes",
        args.message, minutes
    );

    loop {
        if args.print_datetime {
            println!("{} {}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), args.message);
        } else {
            println!("{}", args.message);
        }
        thread::sleep(duration);
    }
}