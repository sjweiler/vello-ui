use crate::{
    BoxConstraints, Color, Event, EventCtx, LayoutCtx, PaintCtx, PaintOp, Point, Rect,
    SemanticsCtx, Size, UiCommand, Widget, WidgetId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MainAxisAlignment {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CrossAxisAlignment {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
}

fn main_of(axis: Axis, size: Size) -> f64 {
    match axis {
        Axis::Horizontal => size.width,
        Axis::Vertical => size.height,
    }
}

fn cross_of(axis: Axis, size: Size) -> f64 {
    match axis {
        Axis::Horizontal => size.height,
        Axis::Vertical => size.width,
    }
}

fn size_from(axis: Axis, main: f64, cross: f64) -> Size {
    match axis {
        Axis::Horizontal => Size::new(main, cross),
        Axis::Vertical => Size::new(cross, main),
    }
}

fn point_from(axis: Axis, main: f64, cross: f64) -> Point {
    match axis {
        Axis::Horizontal => Point::new(main, cross),
        Axis::Vertical => Point::new(cross, main),
    }
}

fn count_as_f64(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

fn main_spacing(alignment: MainAxisAlignment, count: usize, free: f64) -> (f64, f64) {
    match alignment {
        MainAxisAlignment::Center => (free / 2.0, 0.0),
        MainAxisAlignment::End => (free, 0.0),
        MainAxisAlignment::SpaceBetween if count > 1 => (0.0, free / count_as_f64(count - 1)),
        MainAxisAlignment::SpaceAround if count > 0 => {
            let spacing = free / count_as_f64(count);
            (spacing / 2.0, spacing)
        }
        MainAxisAlignment::SpaceEvenly if count > 0 => {
            let spacing = free / count_as_f64(count + 1);
            (spacing, spacing)
        }
        MainAxisAlignment::Start
        | MainAxisAlignment::SpaceBetween
        | MainAxisAlignment::SpaceAround
        | MainAxisAlignment::SpaceEvenly => (0.0, 0.0),
    }
}

fn child_constraints(
    axis: Axis,
    main: f64,
    cross_max: f64,
    stretch_cross: Option<f64>,
) -> BoxConstraints {
    let cross_min = stretch_cross.unwrap_or(0.0);
    let cross_max = stretch_cross.unwrap_or(cross_max);
    BoxConstraints::new(
        size_from(axis, main, cross_min),
        size_from(axis, main, cross_max),
    )
}

fn translate_children(children: &mut [Box<dyn Widget>], bounds: &mut Rect, origin: Point) {
    let dx = origin.x - bounds.x;
    let dy = origin.y - bounds.y;
    bounds.x = origin.x;
    bounds.y = origin.y;
    for child in children {
        let child_bounds = child.bounds();
        child.set_origin(Point::new(child_bounds.x + dx, child_bounds.y + dy));
    }
}

fn route_children(
    children: &mut [Box<dyn Widget>],
    hovered: &mut Option<usize>,
    ctx: &mut EventCtx,
    event: &Event,
) {
    match event {
        Event::PointerMoved(position) => {
            let next = children
                .iter()
                .rposition(|child| child.bounds().contains(*position));
            if next != *hovered {
                if let Some(old) = hovered.take() {
                    children[old].event(ctx, &Event::PointerLeft);
                }
                *hovered = next;
            }
            if let Some(index) = next {
                children[index].event(ctx, event);
            }
        }
        Event::PointerLeft => {
            if let Some(old) = hovered.take() {
                children[old].event(ctx, event);
            }
        }
        Event::PointerDown { position, .. }
        | Event::PointerUp { position, .. }
        | Event::Scroll { position, .. } => {
            if let Some(child) = children
                .iter_mut()
                .rfind(|child| child.bounds().contains(*position))
            {
                child.event(ctx, event);
            }
        }
        _ => {
            for child in children.iter_mut().rev() {
                child.event(ctx, event);
                if ctx.is_handled() {
                    break;
                }
            }
        }
    }
}

fn target_children(
    children: &mut [Box<dyn Widget>],
    ctx: &mut EventCtx,
    event: &Event,
    target: WidgetId,
) -> bool {
    children
        .iter_mut()
        .rev()
        .any(|child| child.event_target(ctx, event, target))
}

fn command_children(
    children: &mut [Box<dyn Widget>],
    mut command: UiCommand,
) -> Result<(), UiCommand> {
    for child in children.iter_mut().rev() {
        match child.apply_command(command) {
            Ok(()) => return Ok(()),
            Err(returned) => command = returned,
        }
    }
    Err(command)
}

fn reconcile_children(current: &mut [Box<dyn Widget>], next: &mut [Box<dyn Widget>]) {
    for (index, next_child) in next.iter_mut().enumerate() {
        let key = next_child.widget_key().cloned();
        let current_child = if let Some(key) = key {
            current
                .iter_mut()
                .find(|child| child.widget_key() == Some(&key))
        } else {
            current
                .get_mut(index)
                .filter(|child| child.widget_key().is_none())
        };
        if let Some(current_child) = current_child {
            current_child.reconcile(next_child.as_mut());
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FlexParams {
    grow: f64,
    shrink: f64,
    basis: Option<f64>,
    align_self: Option<CrossAxisAlignment>,
}

struct FlexRun {
    start: usize,
    allocated: Vec<f64>,
    cross: f64,
}

fn allocate_flex_run(params: &[FlexParams], bases: &[f64], available: f64) -> Vec<f64> {
    let mut allocated = bases.to_vec();
    let used = allocated.iter().sum::<f64>();
    if used < available {
        let grow = params.iter().map(|item| item.grow).sum::<f64>();
        if grow > 0.0 {
            let free = available - used;
            for (size, item) in allocated.iter_mut().zip(params) {
                *size += free * item.grow / grow;
            }
        }
    } else if used > available {
        let weight = allocated
            .iter()
            .zip(params)
            .map(|(size, item)| size * item.shrink)
            .sum::<f64>();
        if weight > 0.0 {
            let overflow = used - available;
            for (size, item) in allocated.iter_mut().zip(params) {
                let item_weight = *size * item.shrink;
                *size = (*size - overflow * item_weight / weight).max(0.0);
            }
        }
    }
    allocated
}

pub struct FlexChild {
    child: Box<dyn Widget>,
    params: FlexParams,
}

impl FlexChild {
    #[must_use]
    pub fn fixed(child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            params: FlexParams {
                grow: 0.0,
                shrink: 0.0,
                basis: None,
                align_self: None,
            },
        }
    }

    #[must_use]
    pub fn flexible(grow: f64, child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            params: FlexParams {
                grow: grow.max(0.0),
                shrink: 1.0,
                basis: Some(0.0),
                align_self: None,
            },
        }
    }

    #[must_use]
    pub fn basis(mut self, basis: f64) -> Self {
        self.params.basis = Some(basis.max(0.0));
        self
    }

    #[must_use]
    pub fn shrink(mut self, shrink: f64) -> Self {
        self.params.shrink = shrink.max(0.0);
        self
    }

    #[must_use]
    pub fn align_self(mut self, alignment: CrossAxisAlignment) -> Self {
        self.params.align_self = Some(alignment);
        self
    }
}

impl From<Box<dyn Widget>> for FlexChild {
    fn from(child: Box<dyn Widget>) -> Self {
        Self {
            child,
            params: FlexParams {
                grow: 0.0,
                shrink: 0.0,
                basis: None,
                align_self: None,
            },
        }
    }
}

pub struct Flex {
    axis: Axis,
    children: Vec<Box<dyn Widget>>,
    params: Vec<FlexParams>,
    gap: f64,
    main_alignment: MainAxisAlignment,
    cross_alignment: CrossAxisAlignment,
    wrap: FlexWrap,
    run_gap: f64,
    overflow: bool,
    bounds: Rect,
    hovered_child: Option<usize>,
}

impl Flex {
    #[must_use]
    pub fn new(axis: Axis, children: impl IntoIterator<Item = FlexChild>) -> Self {
        let (children, params) = children
            .into_iter()
            .map(|child| (child.child, child.params))
            .unzip();
        Self {
            axis,
            children,
            params,
            gap: 0.0,
            main_alignment: MainAxisAlignment::Start,
            cross_alignment: CrossAxisAlignment::Start,
            wrap: FlexWrap::NoWrap,
            run_gap: 0.0,
            overflow: false,
            bounds: Rect::default(),
            hovered_child: None,
        }
    }

    #[must_use]
    pub fn row(children: impl IntoIterator<Item = FlexChild>) -> Self {
        Self::new(Axis::Horizontal, children)
    }

    #[must_use]
    pub fn column(children: impl IntoIterator<Item = FlexChild>) -> Self {
        Self::new(Axis::Vertical, children)
    }

    #[must_use]
    pub fn gap(mut self, gap: f64) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    #[must_use]
    pub fn main_alignment(mut self, alignment: MainAxisAlignment) -> Self {
        self.main_alignment = alignment;
        self
    }

    #[must_use]
    pub fn cross_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.cross_alignment = alignment;
        self
    }

    #[must_use]
    pub fn wrap(mut self, wrap: FlexWrap) -> Self {
        self.wrap = wrap;
        self
    }

    #[must_use]
    pub fn run_gap(mut self, gap: f64) -> Self {
        self.run_gap = gap.max(0.0);
        self
    }

    fn allocate_main(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        constraints: BoxConstraints,
        gap_total: f64,
    ) -> (Vec<f64>, f64) {
        let max_main = main_of(self.axis, constraints.max);
        let min_main = main_of(self.axis, constraints.min);
        let mut allocated = Vec::with_capacity(self.children.len());
        for (child, params) in self.children.iter_mut().zip(&self.params) {
            let main = if let Some(basis) = params.basis {
                basis.min(max_main)
            } else {
                let measured = child.layout(ctx, BoxConstraints::loose(constraints.max));
                main_of(self.axis, measured)
            };
            allocated.push(main.max(0.0));
        }
        let natural_main = allocated.iter().sum::<f64>() + gap_total;
        let has_grow = self.params.iter().any(|params| params.grow > 0.0);
        let container_main = if max_main.is_finite() && has_grow {
            max_main
        } else if max_main.is_finite() {
            natural_main.clamp(min_main, max_main)
        } else {
            natural_main.max(min_main)
        };
        let available = (container_main - gap_total).max(0.0);
        let allocated_sum = allocated.iter().sum::<f64>();
        if allocated_sum < available {
            let total_grow = self.params.iter().map(|params| params.grow).sum::<f64>();
            if total_grow > 0.0 {
                let extra = available - allocated_sum;
                for (size, params) in allocated.iter_mut().zip(&self.params) {
                    *size += extra * params.grow / total_grow;
                }
            }
        } else if allocated_sum > available {
            let total_weight = allocated
                .iter()
                .zip(&self.params)
                .map(|(size, params)| size * params.shrink)
                .sum::<f64>();
            if total_weight > 0.0 {
                let overflow = allocated_sum - available;
                for (size, params) in allocated.iter_mut().zip(&self.params) {
                    let weight = *size * params.shrink;
                    *size = (*size - overflow * weight / total_weight).max(0.0);
                }
            }
        }
        (allocated, container_main)
    }

    fn layout_wrapped(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let max_main = main_of(self.axis, constraints.max);
        let max_cross = cross_of(self.axis, constraints.max);
        let mut bases = Vec::with_capacity(self.children.len());
        for (child, params) in self.children.iter_mut().zip(&self.params) {
            let measured = child.layout(ctx, BoxConstraints::loose(constraints.max));
            bases.push(params.basis.unwrap_or_else(|| main_of(self.axis, measured)));
        }
        let mut ranges = Vec::new();
        let mut start = 0;
        let mut used = 0.0;
        for (index, basis) in bases.iter().copied().enumerate() {
            let next = if index == start {
                basis
            } else {
                used + self.gap + basis
            };
            if index > start && next > max_main {
                ranges.push(start..index);
                start = index;
                used = basis;
            } else {
                used = next;
            }
        }
        if start < bases.len() {
            ranges.push(start..bases.len());
        }

        let mut runs = Vec::with_capacity(ranges.len());
        let mut sizes = vec![Size::ZERO; self.children.len()];
        self.overflow = false;
        for range in ranges {
            let gaps = self.gap * count_as_f64(range.len().saturating_sub(1));
            let available = (max_main - gaps).max(0.0);
            let allocated = allocate_flex_run(
                &self.params[range.clone()],
                &bases[range.clone()],
                available,
            );
            let mut run_cross: f64 = 0.0;
            for (offset, main) in allocated.iter().copied().enumerate() {
                let index = range.start + offset;
                let size = self.children[index]
                    .layout(ctx, child_constraints(self.axis, main, max_cross, None));
                run_cross = run_cross.max(cross_of(self.axis, size));
                sizes[index] = size;
            }
            let run_main = allocated.iter().sum::<f64>() + gaps;
            self.overflow |= run_main > max_main + f64::EPSILON;
            runs.push(FlexRun {
                start: range.start,
                allocated,
                cross: run_cross,
            });
        }

        let run_gaps = self.run_gap * count_as_f64(runs.len().saturating_sub(1));
        let natural_cross = runs.iter().map(|run| run.cross).sum::<f64>() + run_gaps;
        let constrained = constraints.constrain(size_from(self.axis, max_main, natural_cross));
        let container_main = main_of(self.axis, constrained);
        let container_cross = cross_of(self.axis, constrained);
        let mut cross_cursor = 0.0;
        for run in &runs {
            let gaps = self.gap * count_as_f64(run.allocated.len().saturating_sub(1));
            let free = (container_main - run.allocated.iter().sum::<f64>() - gaps).max(0.0);
            let (mut main_cursor, extra_gap) =
                main_spacing(self.main_alignment, run.allocated.len(), free);
            for (offset, main) in run.allocated.iter().copied().enumerate() {
                let index = run.start + offset;
                let alignment = self.params[index]
                    .align_self
                    .unwrap_or(self.cross_alignment);
                if alignment == CrossAxisAlignment::Stretch {
                    sizes[index] = self.children[index].layout(
                        ctx,
                        child_constraints(self.axis, main, max_cross, Some(run.cross)),
                    );
                }
                let child_cross = cross_of(self.axis, sizes[index]);
                let offset_cross = match alignment {
                    CrossAxisAlignment::Start | CrossAxisAlignment::Stretch => 0.0,
                    CrossAxisAlignment::Center => (run.cross - child_cross) / 2.0,
                    CrossAxisAlignment::End => run.cross - child_cross,
                };
                self.children[index].set_origin(point_from(
                    self.axis,
                    main_cursor,
                    cross_cursor + offset_cross,
                ));
                main_cursor += main + self.gap + extra_gap;
            }
            cross_cursor += run.cross + self.run_gap;
        }
        self.bounds.width = constrained.width;
        self.bounds.height = container_cross;
        constrained
    }
}

impl Widget for Flex {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if self.wrap == FlexWrap::Wrap && main_of(self.axis, constraints.max).is_finite() {
            return self.layout_wrapped(ctx, constraints);
        }
        let count = self.children.len();
        let gap_total = self.gap * count_as_f64(count.saturating_sub(1));
        let max_cross = cross_of(self.axis, constraints.max);
        let (allocated, container_main) = self.allocate_main(ctx, constraints, gap_total);

        let mut sizes = Vec::with_capacity(count);
        let mut largest_cross: f64 = 0.0;
        for ((child, main), _) in self.children.iter_mut().zip(&allocated).zip(&self.params) {
            let size = child.layout(ctx, child_constraints(self.axis, *main, max_cross, None));
            largest_cross = largest_cross.max(cross_of(self.axis, size));
            sizes.push(size);
        }
        let natural_cross =
            if self.cross_alignment == CrossAxisAlignment::Stretch && max_cross.is_finite() {
                max_cross
            } else {
                largest_cross
            };
        let container_cross = cross_of(
            self.axis,
            constraints.constrain(size_from(self.axis, container_main, natural_cross)),
        );
        for (index, ((child, main), size)) in self
            .children
            .iter_mut()
            .zip(&allocated)
            .zip(&mut sizes)
            .enumerate()
        {
            let alignment = self.params[index]
                .align_self
                .unwrap_or(self.cross_alignment);
            if alignment == CrossAxisAlignment::Stretch {
                *size = child.layout(
                    ctx,
                    child_constraints(self.axis, *main, max_cross, Some(container_cross)),
                );
            }
        }

        let used_main = allocated.iter().sum::<f64>() + gap_total;
        let free = (container_main - used_main).max(0.0);
        let (mut cursor, extra_gap) = main_spacing(self.main_alignment, count, free);
        self.overflow = used_main > container_main + f64::EPSILON;
        for (index, ((child, size), main)) in self
            .children
            .iter_mut()
            .zip(&sizes)
            .zip(&allocated)
            .enumerate()
        {
            let cross = cross_of(self.axis, *size);
            let alignment = self.params[index]
                .align_self
                .unwrap_or(self.cross_alignment);
            let cross_offset = match alignment {
                CrossAxisAlignment::Start | CrossAxisAlignment::Stretch => 0.0,
                CrossAxisAlignment::Center => (container_cross - cross) / 2.0,
                CrossAxisAlignment::End => container_cross - cross,
            };
            child.set_origin(point_from(self.axis, cursor, cross_offset));
            cursor += main + self.gap + extra_gap;
        }
        let size = constraints.constrain(size_from(self.axis, container_main, container_cross));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }

    fn set_origin(&mut self, origin: Point) {
        translate_children(&mut self.children, &mut self.bounds, origin);
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        route_children(&mut self.children, &mut self.hovered_child, ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        target_children(&mut self.children, ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        command_children(&mut self.children, command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            reconcile_children(&mut self.children, &mut next.children);
            next.hovered_child = self
                .hovered_child
                .filter(|index| *index < next.children.len());
            next.overflow = self.overflow;
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        for child in &self.children {
            child.paint(ctx);
        }
        #[cfg(debug_assertions)]
        if self.overflow {
            ctx.push(PaintOp::StrokeRect {
                rect: self.bounds,
                color: Color::rgb(255, 64, 64),
                width: 2.0,
                radius: 0.0,
            });
        }
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        for child in &self.children {
            child.semantics(ctx);
        }
    }
}

pub struct Spacer {
    size: Size,
    bounds: Rect,
}

impl Spacer {
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self {
            size: Size::new(width, height),
            bounds: Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }
}

impl Widget for Spacer {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = constraints.constrain(self.size);
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, _ctx: &mut EventCtx, _event: &Event) {}
    fn paint(&self, _ctx: &mut PaintCtx<'_>) {}
    fn semantics(&self, _ctx: &mut SemanticsCtx<'_>) {}
}

pub struct SizedBox {
    child: Box<dyn Widget>,
    width: Option<f64>,
    height: Option<f64>,
    fill_width: bool,
    fill_height: bool,
    bounds: Rect,
}

impl SizedBox {
    #[must_use]
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            width: None,
            height: None,
            fill_width: false,
            fill_height: false,
            bounds: Rect::default(),
        }
    }
    #[must_use]
    pub fn width(mut self, width: f64) -> Self {
        self.width = Some(width.max(0.0));
        self
    }
    #[must_use]
    pub fn height(mut self, height: f64) -> Self {
        self.height = Some(height.max(0.0));
        self
    }
    #[must_use]
    pub fn fill_width(mut self) -> Self {
        self.fill_width = true;
        self
    }
    #[must_use]
    pub fn fill_height(mut self) -> Self {
        self.fill_height = true;
        self
    }
}

impl Widget for SizedBox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let width = self.width.or_else(|| {
            (self.fill_width && constraints.max.width.is_finite()).then_some(constraints.max.width)
        });
        let height = self.height.or_else(|| {
            (self.fill_height && constraints.max.height.is_finite())
                .then_some(constraints.max.height)
        });
        let min = Size::new(
            width
                .unwrap_or(constraints.min.width)
                .clamp(constraints.min.width, constraints.max.width),
            height
                .unwrap_or(constraints.min.height)
                .clamp(constraints.min.height, constraints.max.height),
        );
        let max = Size::new(
            width
                .unwrap_or(constraints.max.width)
                .clamp(min.width, constraints.max.width),
            height
                .unwrap_or(constraints.max.height)
                .clamp(min.height, constraints.max.height),
        );
        let size = self.child.layout(ctx, BoxConstraints::new(min, max));
        self.child.set_origin(Point::new(0.0, 0.0));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        let old = self.bounds;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        let child = self.child.bounds();
        self.child.set_origin(Point::new(
            child.x + origin.x - old.x,
            child.y + origin.y - old.y,
        ));
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.child.event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.child.event_target(ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.child.apply_command(command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.child.reconcile(next.child.as_mut());
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.child.paint(ctx);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

pub struct ConstrainedBox {
    child: Box<dyn Widget>,
    min_width: Option<f64>,
    min_height: Option<f64>,
    max_width: Option<f64>,
    max_height: Option<f64>,
    aspect_ratio: Option<f64>,
    bounds: Rect,
}

impl ConstrainedBox {
    #[must_use]
    pub fn new(child: impl Widget + 'static) -> Self {
        Self {
            child: Box::new(child),
            min_width: None,
            min_height: None,
            max_width: None,
            max_height: None,
            aspect_ratio: None,
            bounds: Rect::default(),
        }
    }

    #[must_use]
    pub fn min_width(mut self, value: f64) -> Self {
        self.min_width = Some(value.max(0.0));
        self
    }

    #[must_use]
    pub fn min_height(mut self, value: f64) -> Self {
        self.min_height = Some(value.max(0.0));
        self
    }

    #[must_use]
    pub fn max_width(mut self, value: f64) -> Self {
        self.max_width = Some(value.max(0.0));
        self
    }

    #[must_use]
    pub fn max_height(mut self, value: f64) -> Self {
        self.max_height = Some(value.max(0.0));
        self
    }

    #[must_use]
    pub fn aspect_ratio(mut self, width_over_height: f64) -> Self {
        self.aspect_ratio =
            (width_over_height.is_finite() && width_over_height > 0.0).then_some(width_over_height);
        self
    }

    fn effective_constraints(&self, incoming: BoxConstraints) -> BoxConstraints {
        let max_width = self
            .max_width
            .unwrap_or(incoming.max.width)
            .min(incoming.max.width)
            .max(incoming.min.width);
        let max_height = self
            .max_height
            .unwrap_or(incoming.max.height)
            .min(incoming.max.height)
            .max(incoming.min.height);
        let min_width = self
            .min_width
            .unwrap_or(incoming.min.width)
            .max(incoming.min.width)
            .min(max_width);
        let min_height = self
            .min_height
            .unwrap_or(incoming.min.height)
            .max(incoming.min.height)
            .min(max_height);
        BoxConstraints::new(
            Size::new(min_width, min_height),
            Size::new(max_width, max_height),
        )
    }
}

impl Widget for ConstrainedBox {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let effective = self.effective_constraints(constraints);
        let size = if let Some(ratio) = self.aspect_ratio {
            let mut target = if effective.max.width.is_finite() {
                Size::new(effective.max.width, effective.max.width / ratio)
            } else if effective.max.height.is_finite() {
                Size::new(effective.max.height * ratio, effective.max.height)
            } else {
                self.child.layout(ctx, effective)
            };
            if target.height > effective.max.height {
                target.height = effective.max.height;
                target.width = target.height * ratio;
            }
            if target.width < effective.min.width {
                target.width = effective.min.width;
                target.height = target.width / ratio;
            }
            if target.height < effective.min.height {
                target.height = effective.min.height;
                target.width = target.height * ratio;
            }
            let target = effective.constrain(target);
            self.child.layout(ctx, BoxConstraints::tight(target))
        } else {
            self.child.layout(ctx, effective)
        };
        self.child.set_origin(Point::new(0.0, 0.0));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }

    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        let child = self.child.bounds();
        self.child
            .set_origin(Point::new(child.x + dx, child.y + dy));
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.child.event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.child.event_target(ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.child.apply_command(command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            self.child.reconcile(next.child.as_mut());
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.child.paint(ctx);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.child.semantics(ctx);
    }
}

pub struct Responsive {
    breakpoint: f64,
    children: Vec<Box<dyn Widget>>,
    active: usize,
    bounds: Rect,
}

impl Responsive {
    #[must_use]
    pub fn new(
        breakpoint: f64,
        compact: impl Widget + 'static,
        wide: impl Widget + 'static,
    ) -> Self {
        Self {
            breakpoint: breakpoint.max(0.0),
            children: vec![Box::new(compact), Box::new(wide)],
            active: 0,
            bounds: Rect::default(),
        }
    }

    fn active_mut(&mut self) -> &mut Box<dyn Widget> {
        &mut self.children[self.active]
    }
    fn active_ref(&self) -> &dyn Widget {
        self.children[self.active].as_ref()
    }
}

impl Widget for Responsive {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        self.active = usize::from(constraints.max.width > self.breakpoint);
        let size = self.active_mut().layout(ctx, constraints);
        self.active_mut().set_origin(Point::new(0.0, 0.0));
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        self.bounds.x = origin.x;
        self.bounds.y = origin.y;
        self.active_mut().set_origin(origin);
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        self.active_mut().event(ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        self.active_mut().event_target(ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        self.active_mut().apply_command(command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            reconcile_children(&mut self.children, &mut next.children);
            next.active = self.active;
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.active_ref().paint(ctx);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        self.active_ref().semantics(ctx);
    }
}

pub struct Stack {
    children: Vec<Box<dyn Widget>>,
    horizontal: CrossAxisAlignment,
    vertical: CrossAxisAlignment,
    bounds: Rect,
    hovered_child: Option<usize>,
}

impl Stack {
    #[must_use]
    pub fn new(children: impl IntoIterator<Item = Box<dyn Widget>>) -> Self {
        Self {
            children: children.into_iter().collect(),
            horizontal: CrossAxisAlignment::Start,
            vertical: CrossAxisAlignment::Start,
            bounds: Rect::default(),
            hovered_child: None,
        }
    }
    #[must_use]
    pub fn horizontal_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.horizontal = alignment;
        self
    }
    #[must_use]
    pub fn vertical_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.vertical = alignment;
        self
    }
}

impl Widget for Stack {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let mut natural = Size::ZERO;
        let mut sizes = Vec::with_capacity(self.children.len());
        for child in &mut self.children {
            let size = child.layout(ctx, BoxConstraints::loose(constraints.max));
            natural.width = natural.width.max(size.width);
            natural.height = natural.height.max(size.height);
            sizes.push(size);
        }
        let size = constraints.constrain(natural);
        for (child, child_size) in self.children.iter_mut().zip(sizes) {
            let width = if self.horizontal == CrossAxisAlignment::Stretch {
                size.width
            } else {
                child_size.width
            };
            let height = if self.vertical == CrossAxisAlignment::Stretch {
                size.height
            } else {
                child_size.height
            };
            let actual = if (width - child_size.width).abs() > f64::EPSILON
                || (height - child_size.height).abs() > f64::EPSILON
            {
                child.layout(ctx, BoxConstraints::tight(Size::new(width, height)))
            } else {
                child_size
            };
            let x = match self.horizontal {
                CrossAxisAlignment::Center => (size.width - actual.width) / 2.0,
                CrossAxisAlignment::End => size.width - actual.width,
                _ => 0.0,
            };
            let y = match self.vertical {
                CrossAxisAlignment::Center => (size.height - actual.height) / 2.0,
                CrossAxisAlignment::End => size.height - actual.height,
                _ => 0.0,
            };
            child.set_origin(Point::new(x, y));
        }
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        translate_children(&mut self.children, &mut self.bounds, origin);
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        route_children(&mut self.children, &mut self.hovered_child, ctx, event);
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        target_children(&mut self.children, ctx, event, target)
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        command_children(&mut self.children, command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            reconcile_children(&mut self.children, &mut next.children);
            next.hovered_child = self
                .hovered_child
                .filter(|index| *index < next.children.len());
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        for child in &self.children {
            child.paint(ctx);
        }
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        for child in &self.children {
            child.semantics(ctx);
        }
    }
}

pub struct SplitPane {
    id: WidgetId,
    axis: Axis,
    children: Vec<Box<dyn Widget>>,
    ratio: f64,
    divider: f64,
    min_pane: f64,
    bounds: Rect,
    divider_bounds: Rect,
    dragging: bool,
    hovered_divider: bool,
    hovered_child: Option<usize>,
}

impl SplitPane {
    #[must_use]
    pub fn new(axis: Axis, first: impl Widget + 'static, second: impl Widget + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            axis,
            children: vec![Box::new(first), Box::new(second)],
            ratio: 0.5,
            divider: 6.0,
            min_pane: 48.0,
            bounds: Rect::default(),
            divider_bounds: Rect::default(),
            dragging: false,
            hovered_divider: false,
            hovered_child: None,
        }
    }
    #[must_use]
    pub fn horizontal(first: impl Widget + 'static, second: impl Widget + 'static) -> Self {
        Self::new(Axis::Horizontal, first, second)
    }
    #[must_use]
    pub fn vertical(first: impl Widget + 'static, second: impl Widget + 'static) -> Self {
        Self::new(Axis::Vertical, first, second)
    }
    #[must_use]
    pub fn ratio(mut self, ratio: f64) -> Self {
        self.ratio = ratio.clamp(0.0, 1.0);
        self
    }
    #[must_use]
    pub fn divider_width(mut self, width: f64) -> Self {
        self.divider = width.max(1.0);
        self
    }
    #[must_use]
    pub fn min_pane(mut self, size: f64) -> Self {
        self.min_pane = size.max(0.0);
        self
    }

    fn set_ratio_from_point(&mut self, position: Point) {
        let total = (main_of(self.axis, Size::new(self.bounds.width, self.bounds.height))
            - self.divider)
            .max(0.0);
        if total == 0.0 {
            return;
        }
        let local = match self.axis {
            Axis::Horizontal => position.x - self.bounds.x,
            Axis::Vertical => position.y - self.bounds.y,
        };
        let minimum = self.min_pane.min(total / 2.0);
        self.ratio = local.clamp(minimum, total - minimum) / total;
    }
}

impl Widget for SplitPane {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        let size = if constraints.max.width.is_finite() && constraints.max.height.is_finite() {
            constraints.constrain(constraints.max)
        } else {
            let first = self.children[0].layout(ctx, BoxConstraints::loose(constraints.max));
            let second = self.children[1].layout(ctx, BoxConstraints::loose(constraints.max));
            constraints.constrain(size_from(
                self.axis,
                main_of(self.axis, first) + self.divider + main_of(self.axis, second),
                cross_of(self.axis, first).max(cross_of(self.axis, second)),
            ))
        };
        let main = main_of(self.axis, size);
        let cross = cross_of(self.axis, size);
        let available = (main - self.divider).max(0.0);
        let minimum = self.min_pane.min(available / 2.0);
        let first_main = (available * self.ratio).clamp(minimum, available - minimum);
        self.ratio = if available > 0.0 {
            first_main / available
        } else {
            0.5
        };
        let second_main = available - first_main;
        self.children[0].layout(
            ctx,
            child_constraints(self.axis, first_main, cross, Some(cross)),
        );
        self.children[1].layout(
            ctx,
            child_constraints(self.axis, second_main, cross, Some(cross)),
        );
        self.children[0].set_origin(Point::new(0.0, 0.0));
        self.children[1].set_origin(point_from(self.axis, first_main + self.divider, 0.0));
        self.divider_bounds = match self.axis {
            Axis::Horizontal => Rect::new(first_main, 0.0, self.divider, cross),
            Axis::Vertical => Rect::new(0.0, first_main, cross, self.divider),
        };
        self.bounds.width = size.width;
        self.bounds.height = size.height;
        size
    }
    fn set_origin(&mut self, origin: Point) {
        let dx = origin.x - self.bounds.x;
        let dy = origin.y - self.bounds.y;
        translate_children(&mut self.children, &mut self.bounds, origin);
        self.divider_bounds.x += dx;
        self.divider_bounds.y += dy;
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn event(&mut self, ctx: &mut EventCtx, event: &Event) {
        match event {
            Event::PointerMoved(position) if self.dragging => {
                self.set_ratio_from_point(*position);
                ctx.request_layout();
                ctx.set_handled();
            }
            Event::PointerMoved(position) => {
                let hovered = self.divider_bounds.contains(*position);
                if hovered != self.hovered_divider {
                    self.hovered_divider = hovered;
                    ctx.request_paint();
                }
                if !hovered {
                    route_children(&mut self.children, &mut self.hovered_child, ctx, event);
                }
            }
            Event::PointerDown {
                position,
                button: crate::PointerButton::Primary,
                ..
            } if self.divider_bounds.contains(*position) => {
                self.dragging = true;
                ctx.capture_pointer(self.id);
                ctx.set_handled();
                ctx.request_paint();
            }
            Event::PointerUp {
                position,
                button: crate::PointerButton::Primary,
            } if self.dragging => {
                self.set_ratio_from_point(*position);
                self.dragging = false;
                ctx.release_pointer();
                ctx.set_handled();
                ctx.request_layout();
            }
            Event::PointerLeft => {
                self.hovered_divider = false;
                route_children(&mut self.children, &mut self.hovered_child, ctx, event);
                ctx.request_paint();
            }
            _ => route_children(&mut self.children, &mut self.hovered_child, ctx, event),
        }
    }
    fn widget_id(&self) -> Option<WidgetId> {
        Some(self.id)
    }
    fn event_target(&mut self, ctx: &mut EventCtx, event: &Event, target: WidgetId) -> bool {
        if target == self.id {
            self.event(ctx, event);
            true
        } else {
            target_children(&mut self.children, ctx, event, target)
        }
    }
    fn apply_command(&mut self, command: UiCommand) -> Result<(), UiCommand> {
        command_children(&mut self.children, command)
    }
    fn reconcile(&mut self, next: &mut dyn Widget) {
        if let Some(next) = next.as_any_mut().downcast_mut::<Self>() {
            next.id = self.id;
            next.ratio = self.ratio;
            next.dragging = self.dragging;
            next.hovered_divider = self.hovered_divider;
            next.hovered_child = self.hovered_child;
            reconcile_children(&mut self.children, &mut next.children);
        }
    }
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        self.children[0].paint(ctx);
        ctx.push(PaintOp::FillRect {
            rect: self.divider_bounds,
            color: if self.dragging || self.hovered_divider {
                ctx.theme().focus
            } else {
                Color::rgba(255, 255, 255, 28)
            },
            radius: self.divider / 2.0,
        });
        self.children[1].paint(ctx);
    }
    fn semantics(&self, ctx: &mut SemanticsCtx<'_>) {
        for child in &self.children {
            child.semantics(ctx);
        }
    }
}
