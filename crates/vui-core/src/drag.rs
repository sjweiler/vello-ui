use crate::{Point, WidgetId};
use std::{any::Any, fmt, rc::Rc};

/// Type-erased application data carried by a drag session.
#[derive(Clone)]
pub struct DragPayload {
    value: Rc<dyn Any>,
    type_name: &'static str,
}

impl DragPayload {
    #[must_use]
    pub fn new<T: Any>(value: T) -> Self {
        Self {
            value: Rc::new(value),
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
}

impl fmt::Debug for DragPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DragPayload")
            .field("type_name", &self.type_name)
            .finish_non_exhaustive()
    }
}

impl PartialEq for DragPayload {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.value, &other.value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragOperation {
    Copy,
    Move,
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragOperations(u8);

impl DragOperations {
    pub const COPY: Self = Self(1);
    pub const MOVE: Self = Self(1 << 1);
    pub const LINK: Self = Self(1 << 2);
    pub const ALL: Self = Self(Self::COPY.0 | Self::MOVE.0 | Self::LINK.0);

    #[must_use]
    pub const fn contains(self, operation: DragOperation) -> bool {
        let bit = match operation {
            DragOperation::Copy => Self::COPY.0,
            DragOperation::Move => Self::MOVE.0,
            DragOperation::Link => Self::LINK.0,
        };
        self.0 & bit != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragPreview {
    pub label: String,
}

impl DragPreview {
    #[must_use]
    pub fn label(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DragRequest {
    Start {
        source: WidgetId,
        payload: DragPayload,
        allowed: DragOperations,
        preview: DragPreview,
        position: Point,
    },
    Accept {
        target: WidgetId,
        operation: DragOperation,
    },
    Cancel,
}
