//! Versioned, crash-resistant desktop window session persistence.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use vui_core::{Event, MonitorInfo, Point, Size, WindowDescriptor};

pub const SESSION_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionWindow {
    pub key: String,
    pub position: Option<[f64; 2]>,
    pub size: [f64; 2],
    pub monitor: Option<String>,
    pub maximized: bool,
    pub fullscreen: bool,
    pub open: bool,
}

impl SessionWindow {
    #[must_use]
    pub fn new(key: impl Into<String>, size: Size) -> Self {
        Self {
            key: key.into(),
            position: None,
            size: [size.width, size.height],
            monitor: None,
            maximized: false,
            fullscreen: false,
            open: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub version: u32,
    pub windows: Vec<SessionWindow>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            windows: Vec::new(),
        }
    }
}

pub struct SessionStore {
    path: PathBuf,
    state: SessionState,
    dirty_since: Option<Instant>,
    debounce: Duration,
}

impl SessionStore {
    /// Opens an existing session or starts with an empty state.
    ///
    /// # Errors
    ///
    /// Returns an error when an existing file cannot be read or contains
    /// invalid JSON. Unknown session versions are safely treated as empty.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let state = match fs::read(&path) {
            Ok(bytes) => {
                let decoded: SessionState = serde_json::from_slice(&bytes)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                if decoded.version == SESSION_VERSION {
                    decoded
                } else {
                    SessionState::default()
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => SessionState::default(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            state,
            dirty_since: None,
            debounce: Duration::from_millis(500),
        })
    }

    #[must_use]
    pub const fn state(&self) -> &SessionState {
        &self.state
    }

    #[must_use]
    pub fn window(&self, key: &str) -> Option<&SessionWindow> {
        self.state.windows.iter().find(|window| window.key == key)
    }

    #[must_use]
    pub const fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    pub fn record_event(&mut self, key: &str, event: &Event, now: Instant) {
        let window = self.window_mut(key, Size::new(640.0, 360.0));
        match event {
            Event::WindowMoved(position) => window.position = Some([position.x, position.y]),
            Event::WindowResized(size) => window.size = [size.width, size.height],
            Event::MonitorChanged(monitor) => window.monitor.clone_from(&monitor.name),
            _ => return,
        }
        self.mark_dirty(now);
    }

    pub fn set_open(&mut self, key: &str, open: bool, now: Instant) {
        self.window_mut(key, Size::new(640.0, 360.0)).open = open;
        self.mark_dirty(now);
    }

    pub fn set_window_state(&mut self, key: &str, maximized: bool, fullscreen: bool, now: Instant) {
        let window = self.window_mut(key, Size::new(640.0, 360.0));
        window.maximized = maximized;
        window.fullscreen = fullscreen;
        self.mark_dirty(now);
    }

    #[must_use]
    pub fn restore_descriptor(
        &self,
        key: &str,
        mut descriptor: WindowDescriptor,
        monitors: &[MonitorInfo],
    ) -> WindowDescriptor {
        let Some(saved) = self.state.windows.iter().find(|window| window.key == key) else {
            return descriptor;
        };
        descriptor.size = Size::new(saved.size[0].max(1.0), saved.size[1].max(1.0));
        if let Some(position) = saved.position {
            descriptor.position = Some(clamp_position(
                Point::new(position[0], position[1]),
                descriptor.size,
                saved.monitor.as_deref(),
                monitors,
            ));
        }
        descriptor
    }

    #[must_use]
    pub fn was_open(&self, key: &str) -> bool {
        self.state
            .windows
            .iter()
            .find(|window| window.key == key)
            .is_some_and(|window| window.open)
    }

    /// Writes a dirty session once its debounce interval has elapsed.
    ///
    /// # Errors
    ///
    /// Returns file-system or JSON serialization errors.
    pub fn flush_if_due(&mut self, now: Instant) -> io::Result<bool> {
        if self
            .dirty_since
            .is_none_or(|dirty| now.saturating_duration_since(dirty) < self.debounce)
        {
            return Ok(false);
        }
        self.save_now()?;
        Ok(true)
    }

    /// Atomically replaces the session file after syncing the temporary file.
    ///
    /// # Errors
    ///
    /// Returns file-system or JSON serialization errors.
    pub fn save_now(&mut self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&self.state)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let temporary = temporary_path(&self.path);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &self.path)?;
        self.dirty_since = None;
        Ok(())
    }

    fn window_mut(&mut self, key: &str, size: Size) -> &mut SessionWindow {
        if let Some(index) = self
            .state
            .windows
            .iter()
            .position(|window| window.key == key)
        {
            return &mut self.state.windows[index];
        }
        self.state.windows.push(SessionWindow::new(key, size));
        self.state.windows.last_mut().expect("window was inserted")
    }

    fn mark_dirty(&mut self, now: Instant) {
        self.dirty_since.get_or_insert(now);
    }
}

fn clamp_position(
    position: Point,
    size: Size,
    preferred_monitor: Option<&str>,
    monitors: &[MonitorInfo],
) -> Point {
    let Some(monitor) = monitors
        .iter()
        .find(|monitor| monitor.name.as_deref() == preferred_monitor)
        .or_else(|| monitors.first())
    else {
        return position;
    };
    let width = size.width.min(monitor.size.width);
    let height = size.height.min(monitor.size.height);
    Point::new(
        position.x.clamp(
            monitor.position.x,
            monitor.position.x + monitor.size.width - width,
        ),
        position.y.clamp(
            monitor.position.y,
            monitor.position.y + monitor.size.height - height,
        ),
    )
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_versioned_state_and_restores_onscreen() {
        let path = std::env::temp_dir().join(format!(
            "vui-session-{}.json",
            vui_core::WindowId::next().raw()
        ));
        let start = Instant::now();
        let mut store = SessionStore::open(&path)
            .expect("open session")
            .with_debounce(Duration::ZERO);
        store.record_event(
            "main",
            &Event::WindowMoved(Point::new(9000.0, 9000.0)),
            start,
        );
        store.record_event(
            "main",
            &Event::WindowResized(Size::new(800.0, 600.0)),
            start,
        );
        store.set_window_state("main", true, false, start);
        assert!(store.flush_if_due(start).expect("flush session"));

        let restored = SessionStore::open(&path).expect("reload session");
        let monitor = MonitorInfo {
            name: Some("primary".to_owned()),
            position: Point::new(0.0, 0.0),
            size: Size::new(1920.0, 1080.0),
            scale_factor: 1.0,
        };
        let descriptor = restored.restore_descriptor(
            "main",
            WindowDescriptor::new("Main", Size::new(10.0, 10.0)),
            &[monitor],
        );
        assert_eq!(descriptor.size, Size::new(800.0, 600.0));
        assert_eq!(descriptor.position, Some(Point::new(1120.0, 480.0)));
        assert!(restored.state().windows[0].maximized);
        fs::remove_file(path).expect("remove test session");
    }

    #[test]
    fn unknown_versions_start_empty() {
        let path = std::env::temp_dir().join(format!(
            "vui-session-version-{}.json",
            vui_core::WindowId::next().raw()
        ));
        fs::write(&path, br#"{"version":999,"windows":[]}"#).expect("write session");
        let store = SessionStore::open(&path).expect("open session");
        assert_eq!(store.state(), &SessionState::default());
        fs::remove_file(path).expect("remove test session");
    }
}
