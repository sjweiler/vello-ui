//! Replaceable native desktop-service boundary and deterministic mock.

use std::{collections::VecDeque, fmt, sync::Arc, time::Duration};
use vui_core::{PlatformEvent, PlatformRequest, SystemPreferences, WindowId};

type ResponseCallback = dyn Fn(WindowId, PlatformEvent) + Send + Sync;
pub type PlatformResponder = Arc<ResponseCallback>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformError {
    message: String,
}

impl PlatformError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PlatformError {}

pub trait PlatformServices {
    /// Executes or schedules one platform request.
    ///
    /// # Errors
    ///
    /// Returns an error when the backend cannot accept the request. Deferred
    /// failures are returned through `PlatformEvent::Failed`.
    fn request(
        &mut self,
        origin: WindowId,
        request: PlatformRequest,
        respond: PlatformResponder,
    ) -> Result<(), PlatformError>;

    fn preferences(&self) -> SystemPreferences {
        SystemPreferences::default()
    }

    /// Drains unsolicited operating-system events, such as tray activation.
    fn poll_events(&mut self) -> Vec<(WindowId, PlatformEvent)> {
        Vec::new()
    }

    /// Requests periodic host polling while an integration needs it.
    fn poll_interval(&self) -> Option<Duration> {
        None
    }
}

struct PendingResponse {
    origin: WindowId,
    respond: PlatformResponder,
}

#[derive(Default)]
pub struct MockPlatformServices {
    requests: Vec<(WindowId, PlatformRequest)>,
    pending: VecDeque<PendingResponse>,
    preferences: SystemPreferences,
    events: VecDeque<(WindowId, PlatformEvent)>,
}

impl MockPlatformServices {
    #[must_use]
    pub fn requests(&self) -> &[(WindowId, PlatformRequest)] {
        &self.requests
    }

    pub fn take_requests(&mut self) -> Vec<(WindowId, PlatformRequest)> {
        std::mem::take(&mut self.requests)
    }

    pub fn respond_next(&mut self, event: PlatformEvent) -> bool {
        let Some(pending) = self.pending.pop_front() else {
            return false;
        };
        (pending.respond)(pending.origin, event);
        true
    }

    pub fn set_preferences(&mut self, preferences: SystemPreferences) {
        self.preferences = preferences;
    }

    pub fn push_event(&mut self, window: WindowId, event: PlatformEvent) {
        self.events.push_back((window, event));
    }
}

impl PlatformServices for MockPlatformServices {
    fn request(
        &mut self,
        origin: WindowId,
        request: PlatformRequest,
        respond: PlatformResponder,
    ) -> Result<(), PlatformError> {
        self.requests.push((origin, request));
        self.pending.push_back(PendingResponse { origin, respond });
        Ok(())
    }

    fn preferences(&self) -> SystemPreferences {
        self.preferences.clone()
    }

    fn poll_events(&mut self) -> Vec<(WindowId, PlatformEvent)> {
        self.events.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn mock_records_requests_and_delivers_typed_responses() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let output = received.clone();
        let responder: PlatformResponder = Arc::new(move |window, event| {
            output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((window, event));
        });
        let mut services = MockPlatformServices::default();
        let window = WindowId::next();
        services
            .request(
                window,
                PlatformRequest::OpenUri("https://example.com".to_owned()),
                responder,
            )
            .expect("accept request");
        assert_eq!(services.requests().len(), 1);
        assert!(services.respond_next(PlatformEvent::OpenUrls(vec![
            "https://example.com".to_owned()
        ])));
        let received = received
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, window);
    }
}
