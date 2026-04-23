# heartbeat

A tiny CLI tool that prints a message on a schedule — plain intervals or aligned to the wall clock.

## Install

```bash
cargo install --path .
```

## Usage

```
heartbeat [OPTIONS] <INTERVAL> <MESSAGE>
```

### Intervals

**Plain** — fires N after start, then every N:
```
30m       30 minutes from now, then every 30m
2h30m     2h 30m from now, then every 2h30m
500ms     every 500ms
```

**Aligned (@)** — fires on wall-clock multiples from midnight:
```
@15m      :00, :15, :30, :45 of every hour
@1h       every hour at :00
@2h30m    00:00, 02:30, 05:00 … 22:30 daily
```

**Aligned with offset** — shifts the anchor:
```
@1h+15m   01:15, 02:15, 03:15 … (fires at :15 past every hour)
@30m+5m   :05 and :35 of every hour
```

### Options

| Flag | Description |
|------|-------------|
| `-p, --print-datetime` | Prefix with ISO-8601 timestamp |
| `--no-time-prefix` | Disable `[HH:MM]` time prefix |
| `--time-prefix-utc` | Use UTC for time prefix |

### Examples

```bash
# Basic heartbeat every 30 minutes
heartbeat 30m "still alive"

# Timestamped pings on quarter-hours
heartbeat -p @15m "ping"

# Hourly report at 15 minutes past each hour
heartbeat @1h+15m "hourly report"

# Fast tick every 500ms
heartbeat 500ms "tick"
```

## For AI Agents

Keep a parent process alive during long-running tasks:

```bash
heartbeat @5m "agent heartbeat" &
HB_PID=$!
# ... do long work ...
kill $HB_PID
```

## Build

```bash
cargo build --release
```
