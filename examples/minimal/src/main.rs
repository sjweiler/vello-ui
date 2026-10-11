use vui_core::{Button, Column, Insets, Label, Padding, Widget};

fn view() -> impl Widget {
    Padding::new(
        Insets::all(24.0),
        Column::new([
            Box::new(Label::new("Hello from VUI")) as Box<dyn Widget>,
            Box::new(Button::new("Continue", || {})),
        ]),
    )
}

fn main() -> anyhow::Result<()> {
    vui_winit::run("VUI minimal application", view())
}
