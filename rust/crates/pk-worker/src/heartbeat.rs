//! The liveness file: the loop touches it before every claim and after every job (`worker/__main__.py`
//! `heartbeat`), and `pk-worker probe --max-age 20m` reads its age. The probe replaces the manifest's
//! `sh -c 'find … -mmin -20'`, which a distroless image cannot run. A job that hangs stops the touches, so the probe
//! fails and the pod restarts: that is the point, so nothing touches the file on a timer.

use std::fs::OpenOptions;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Create the file if needed and set its modification time to now.
pub fn touch(path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let f = OpenOptions::new().create(true).append(true).open(path)?;
    f.set_modified(SystemTime::now())
}

/// How long ago the file was touched; an error when it does not exist.
pub fn age(path: &Path) -> io::Result<Duration> {
    let modified = std::fs::metadata(path)?.modified()?;
    Ok(SystemTime::now().duration_since(modified).unwrap_or_default())
}

/// `20m`, `90s`, `1h`, or plain seconds.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (num, mult) = match s.char_indices().last() {
        Some((i, 's')) => (&s[..i], 1),
        Some((i, 'm')) => (&s[..i], 60),
        Some((i, 'h')) => (&s[..i], 3600),
        _ => (s, 1),
    };
    let n: u64 = num
        .trim()
        .parse()
        .map_err(|_| format!("not a duration: {s:?} (e.g. 20m, 90s, 1h)"))?;
    n.checked_mul(mult)
        .map(Duration::from_secs)
        .ok_or_else(|| format!("too long: {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_then_age() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a").join("heartbeat");
        assert!(age(&p).is_err());
        touch(&p).unwrap();
        assert!(age(&p).unwrap() < Duration::from_secs(5));
        let old = SystemTime::now() - Duration::from_secs(3600);
        OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(age(&p).unwrap() >= Duration::from_secs(3599));
        touch(&p).unwrap();
        assert!(age(&p).unwrap() < Duration::from_secs(5));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("20m").unwrap(), Duration::from_secs(1200));
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("15").unwrap(), Duration::from_secs(15));
        assert!(parse_duration("soon").is_err());
    }
}
