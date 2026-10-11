#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::new(origin.x, origin.y, size.width, size.height)
    }

    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.y >= self.y
            && point.x < self.x + self.width
            && point.y < self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Insets {
    #[must_use]
    pub const fn all(value: f64) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    #[must_use]
    pub const fn horizontal(self) -> f64 {
        self.left + self.right
    }

    #[must_use]
    pub const fn vertical(self) -> f64 {
        self.top + self.bottom
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxConstraints {
    pub min: Size,
    pub max: Size,
}

impl BoxConstraints {
    #[must_use]
    pub fn new(min: Size, max: Size) -> Self {
        debug_assert!(min.width <= max.width && min.height <= max.height);
        Self { min, max }
    }

    #[must_use]
    pub fn tight(size: Size) -> Self {
        Self::new(size, size)
    }

    #[must_use]
    pub fn loose(max: Size) -> Self {
        Self::new(Size::ZERO, max)
    }

    #[must_use]
    pub fn constrain(self, size: Size) -> Size {
        Size::new(
            size.width.clamp(self.min.width, self.max.width),
            size.height.clamp(self.min.height, self.max.height),
        )
    }

    #[must_use]
    pub fn shrink(self, width: f64, height: f64) -> Self {
        let shrink = |value: f64, amount: f64| (value - amount).max(0.0);
        Self::new(
            Size::new(
                shrink(self.min.width, width),
                shrink(self.min.height, height),
            ),
            Size::new(
                shrink(self.max.width, width),
                shrink(self.max.height, height),
            ),
        )
    }
}
