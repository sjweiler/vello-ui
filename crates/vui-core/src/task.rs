use crate::{Event, EventCtx, WidgetId};
use std::{
    any::Any,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskId(u64);

impl TaskId {
    #[must_use]
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone)]
pub struct TaskPayload {
    value: Arc<dyn Any + Send + Sync>,
    type_name: &'static str,
}

impl TaskPayload {
    #[must_use]
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Self {
            value: Arc::new(value),
            type_name: std::any::type_name::<T>(),
        }
    }

    #[must_use]
    pub fn is<T: Any>(&self) -> bool {
        self.value.is::<T>()
    }

    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.value.downcast_ref()
    }

    #[must_use]
    pub fn downcast_arc<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        Arc::downcast::<T>(self.value.clone()).ok()
    }
}

impl fmt::Debug for TaskPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskPayload")
            .field("type_name", &self.type_name)
            .finish_non_exhaustive()
    }
}

impl PartialEq for TaskPayload {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.value, &other.value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TaskUpdate {
    Progress {
        id: TaskId,
        fraction: Option<f64>,
        payload: TaskPayload,
    },
    Completed {
        id: TaskId,
        payload: TaskPayload,
    },
    Failed {
        id: TaskId,
        error: String,
    },
    Cancelled {
        id: TaskId,
    },
}

impl TaskUpdate {
    #[must_use]
    pub const fn id(&self) -> TaskId {
        match self {
            Self::Progress { id, .. }
            | Self::Completed { id, .. }
            | Self::Failed { id, .. }
            | Self::Cancelled { id } => *id,
        }
    }

    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        !matches!(self, Self::Progress { .. })
    }
}

#[derive(Clone)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

type Reporter = Arc<dyn Fn(TaskUpdate) + Send + Sync>;
type Job = Box<dyn FnOnce(TaskContext) -> Result<TaskPayload, String> + Send>;

pub struct TaskContext {
    id: TaskId,
    cancellation: CancellationToken,
    reporter: Reporter,
}

impl TaskContext {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn report<T: Any + Send + Sync>(&self, fraction: Option<f64>, payload: T) {
        if self.is_cancelled() {
            return;
        }
        (self.reporter)(TaskUpdate::Progress {
            id: self.id,
            fraction: fraction.map(|value| value.clamp(0.0, 1.0)),
            payload: TaskPayload::new(payload),
        });
    }
}

pub struct TaskRequest {
    id: TaskId,
    owner: WidgetId,
    cancellation: CancellationToken,
    job: Job,
}

impl TaskRequest {
    pub(crate) fn new<T, F>(owner: WidgetId, job: F) -> Self
    where
        T: Any + Send + Sync,
        F: FnOnce(TaskContext) -> Result<T, String> + Send + 'static,
    {
        let id = TaskId::next();
        Self {
            id,
            owner,
            cancellation: CancellationToken::new(),
            job: Box::new(move |ctx| job(ctx).map(TaskPayload::new)),
        }
    }

    #[must_use]
    pub const fn id(&self) -> TaskId {
        self.id
    }

    #[must_use]
    pub const fn owner(&self) -> WidgetId {
        self.owner
    }

    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn run(self, reporter: impl Fn(TaskUpdate) + Send + Sync + 'static) {
        let reporter: Reporter = Arc::new(reporter);
        let context = TaskContext {
            id: self.id,
            cancellation: self.cancellation.clone(),
            reporter: reporter.clone(),
        };
        if context.is_cancelled() {
            reporter(TaskUpdate::Cancelled { id: self.id });
            return;
        }
        let result = catch_unwind(AssertUnwindSafe(|| (self.job)(context)));
        let update = if self.cancellation.is_cancelled() {
            TaskUpdate::Cancelled { id: self.id }
        } else {
            match result {
                Ok(Ok(payload)) => TaskUpdate::Completed {
                    id: self.id,
                    payload,
                },
                Ok(Err(error)) => TaskUpdate::Failed { id: self.id, error },
                Err(_) => TaskUpdate::Failed {
                    id: self.id,
                    error: "background task panicked".to_owned(),
                },
            }
        };
        reporter(update);
    }
}

impl fmt::Debug for TaskRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskRequest")
            .field("id", &self.id)
            .field("owner", &self.owner)
            .field("cancellation", &self.cancellation)
            .finish_non_exhaustive()
    }
}

pub(crate) enum TaskCommand {
    Spawn(TaskRequest),
    Cancel(TaskId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ResourceState<T> {
    Idle,
    Loading { task: TaskId, progress: Option<f64> },
    Ready(Arc<T>),
    Error(String),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resource<T> {
    state: ResourceState<T>,
}

impl<T> Default for Resource<T> {
    fn default() -> Self {
        Self {
            state: ResourceState::Idle,
        }
    }
}

impl<T: Any + Send + Sync> Resource<T> {
    #[must_use]
    pub const fn state(&self) -> &ResourceState<T> {
        &self.state
    }

    #[must_use]
    pub fn value(&self) -> Option<&T> {
        match &self.state {
            ResourceState::Ready(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn progress(&self) -> Option<f64> {
        match &self.state {
            ResourceState::Loading { progress, .. } => *progress,
            _ => None,
        }
    }

    #[must_use]
    pub fn error(&self) -> Option<&str> {
        match &self.state {
            ResourceState::Error(error) => Some(error),
            _ => None,
        }
    }

    pub fn begin(&mut self, task: TaskId) {
        self.state = ResourceState::Loading {
            task,
            progress: None,
        };
    }

    pub fn cancel(&mut self, ctx: &mut EventCtx) {
        if let ResourceState::Loading { task, .. } = &self.state {
            ctx.cancel_task(*task);
            self.state = ResourceState::Cancelled;
        }
    }

    pub fn handle_event(&mut self, event: &Event) -> bool {
        let ResourceState::Loading { task, .. } = &self.state else {
            return false;
        };
        let task = *task;
        match event {
            Event::TaskProgress { id, fraction, .. } if *id == task => {
                self.state = ResourceState::Loading {
                    task,
                    progress: *fraction,
                };
            }
            Event::TaskCompleted { id, payload } if *id == task => {
                self.state = payload.downcast_arc::<T>().map_or_else(
                    || ResourceState::Error("task returned an unexpected type".to_owned()),
                    ResourceState::Ready,
                );
            }
            Event::TaskFailed { id, error } if *id == task => {
                self.state = ResourceState::Error(error.clone());
            }
            Event::TaskCancelled { id } if *id == task => {
                self.state = ResourceState::Cancelled;
            }
            _ => return false,
        }
        true
    }
}
