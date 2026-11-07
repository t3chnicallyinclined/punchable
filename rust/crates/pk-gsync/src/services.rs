//! The things a run touches outside its own logic, so a test can stand in for each: the clock, sleeping, the browser,
//! starting the memo processor, the free disk, the owner-only file lock and the two Google endpoints. [`Services::system`]
//! is the real set.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use chrono::{DateTime, Utc};
use pk_core::time::{Clock, System};

use crate::config::Config;
use crate::error::Result;

/// Google's token endpoint.
pub const GOOGLE_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
/// Google's API root.
pub const GOOGLE_ROOT: &str = "https://www.googleapis.com";

/// Starts the memo processor.
pub type SpawnFn = Box<dyn Fn(&Config) -> Result<()>>;

/// Everything outside the logic.
pub struct Services {
    /// `time.time()` and `datetime.now(utc)`.
    pub clock: Box<dyn Clock>,
    /// `time.sleep`.
    pub sleep: Box<dyn Fn(Duration)>,
    /// `webbrowser.open`.
    pub opener: Box<dyn Fn(&str)>,
    /// Start `python -m gsync.memos process`, detached.
    pub spawn: SpawnFn,
    /// Free megabytes on the disk holding the path.
    pub free_mb: Box<dyn Fn(&Path) -> u64>,
    /// Make a file owner-only (`icacls` on Windows, mode 600 elsewhere); whether it worked.
    pub restrict: Box<dyn Fn(&Path) -> bool>,
    /// The OAuth endpoint Google refresh tokens are exchanged at.
    pub google_token_uri: String,
    /// The Drive API root (`https://www.googleapis.com`).
    pub drive_root: String,
}

impl Services {
    /// The real services. `user` is the account `icacls` grants (`USERNAME`).
    pub fn system(user: Option<String>) -> Self {
        Self {
            clock: Box::new(System),
            sleep: Box::new(std::thread::sleep),
            opener: Box::new(open_browser),
            spawn: Box::new(spawn_processor),
            free_mb: Box::new(free_mb),
            restrict: Box::new(move |p| restrict(p, user.as_deref())),
            google_token_uri: GOOGLE_TOKEN_URI.to_string(),
            drive_root: GOOGLE_ROOT.to_string(),
        }
    }

    /// `time.time()`.
    pub fn now_secs(&self) -> f64 {
        self.clock.now().timestamp_millis() as f64 / 1000.0
    }

    /// `datetime.now(timezone.utc)`.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
}

/// Owner-only: drop inherited ACEs and grant the current user full control (Windows); mode 600 elsewhere.
pub fn restrict(p: &Path, user: Option<&str>) -> bool {
    if cfg!(windows) {
        let Some(user) = user else { return false };
        Command::new("icacls")
            .arg(p)
            .args(["/inheritance:r", "/grant:r", &format!("{user}:F")])
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success())
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600)).is_ok()
        }
        #[cfg(not(unix))]
        false
    }
}

fn open_browser(url: &str) {
    let _ = if cfg!(windows) {
        Command::new("cmd")
            .args(["/c", "start", "", url])
            .stdin(Stdio::null())
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
}

/// Free space on the disk holding `p` (or its nearest existing parent), in megabytes; unknown counts as plenty.
pub fn free_mb(p: &Path) -> u64 {
    let mut q: PathBuf = p.to_path_buf();
    while !q.exists() && q.parent().is_some_and(|x| x != q) {
        q = q.parent().map(Path::to_path_buf).unwrap_or_default();
    }
    let canon = std::fs::canonicalize(&q).unwrap_or(q);
    let shown = canon.to_string_lossy().into_owned();
    let canon = PathBuf::from(shown.strip_prefix(r"\\?\").unwrap_or(&shown));
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| canon.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map_or(u64::MAX / 2, |d| d.available_space() / 1_000_000)
}

/// Start `python -m gsync.memos process` detached, in the Python that has faster-whisper (`spawn_processor`). The
/// scheduled task's working folder is the repo root, which is where the Python finds the `gsync` package.
pub fn spawn_processor(cfg: &Config) -> Result<()> {
    let desk = std::env::current_dir().unwrap_or_default();
    let py = if Path::new(&cfg.whisper_python).is_file() {
        cfg.whisper_python.clone()
    } else {
        "python".to_string()
    };
    let log = cfg.comedy.join("db").join("punchable").join("logs").join("memos.log");
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let lf = std::fs::OpenOptions::new().create(true).append(true).open(&log)?;
    let mut cmd = Command::new(py);
    cmd.args(["-m", "gsync.memos", "process"])
        .current_dir(desk)
        .stdin(Stdio::null())
        .stdout(lf.try_clone()?)
        .stderr(lf)
        .env("PYTHONUTF8", "1")
        .env("COMEDY_DIR", &cfg.comedy)
        .env("PUNCHABLE_API", &cfg.api);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        cmd.creation_flags(0x0000_0200 | 0x0800_0000);
    }
    cmd.spawn()?;
    Ok(())
}
