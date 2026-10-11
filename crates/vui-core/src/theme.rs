use crate::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypographyTokens {
    pub body: f32,
    pub label: f32,
    pub title: f32,
    pub caption: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacingTokens {
    pub xs: f64,
    pub sm: f64,
    pub md: f64,
    pub lg: f64,
    pub xl: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiusTokens {
    pub sm: f64,
    pub md: f64,
    pub lg: f64,
    pub pill: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderTokens {
    pub thin: f64,
    pub strong: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationTokens {
    pub low_offset: f64,
    pub high_offset: f64,
    pub shadow: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensityTokens {
    pub control_height: f64,
    pub compact_control_height: f64,
    pub control_padding_x: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorTokens {
    pub window: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub text: Color,
    pub text_muted: Color,
    pub border: Color,
    pub focus: Color,
    pub primary: Color,
    pub primary_hover: Color,
    pub primary_pressed: Color,
    pub secondary: Color,
    pub secondary_hover: Color,
    pub destructive: Color,
    pub destructive_hover: Color,
    pub disabled: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlStateStyle {
    pub background: Color,
    pub foreground: Color,
    pub border: Color,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlState {
    #[default]
    Normal,
    Hovered,
    Pressed,
    Selected,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonStyle {
    pub normal: ControlStateStyle,
    pub hovered: ControlStateStyle,
    pub pressed: ControlStateStyle,
    pub selected: ControlStateStyle,
    pub disabled: ControlStateStyle,
    pub radius: f64,
    pub border_width: f64,
    pub height: f64,
    pub min_width: f64,
    pub padding_x: f64,
    pub text_size: f32,
}

impl ButtonStyle {
    #[must_use]
    pub const fn resolve(self, state: ControlState) -> ControlStateStyle {
        match state {
            ControlState::Normal => self.normal,
            ControlState::Hovered => self.hovered,
            ControlState::Pressed => self.pressed,
            ControlState::Selected => self.selected,
            ControlState::Disabled => self.disabled,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Ghost,
    Destructive,
    Icon,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComponentStyles {
    pub primary_button: ButtonStyle,
    pub secondary_button: ButtonStyle,
    pub ghost_button: ButtonStyle,
    pub destructive_button: ButtonStyle,
    pub icon_button: ButtonStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub mode: ThemeMode,
    pub colors: ColorTokens,
    pub typography: TypographyTokens,
    pub spacing: SpacingTokens,
    pub radii: RadiusTokens,
    pub borders: BorderTokens,
    pub elevation: ElevationTokens,
    pub density: DensityTokens,
    pub components: ComponentStyles,
    // Compatibility aliases for the initial public theme fields.
    pub window: Color,
    pub text: Color,
    pub button: Color,
    pub button_hover: Color,
    pub button_pressed: Color,
    pub button_disabled: Color,
    pub focus: Color,
}

#[derive(Clone, Copy)]
struct ButtonPalette {
    normal: Color,
    hover: Color,
    pressed: Color,
    foreground: Color,
}

fn make_button_style(
    colors: ColorTokens,
    typography: TypographyTokens,
    radii: RadiusTokens,
    borders: BorderTokens,
    density: DensityTokens,
    palette: ButtonPalette,
) -> ButtonStyle {
    let state = |background, foreground, border| ControlStateStyle {
        background,
        foreground,
        border,
    };
    ButtonStyle {
        normal: state(palette.normal, palette.foreground, palette.normal),
        hovered: state(palette.hover, palette.foreground, palette.hover),
        pressed: state(palette.pressed, palette.foreground, palette.pressed),
        selected: state(colors.focus, Color::WHITE, colors.focus),
        disabled: state(colors.disabled, colors.text_muted, colors.disabled),
        radius: radii.md,
        border_width: borders.thin,
        height: density.control_height,
        min_width: 88.0,
        padding_x: density.control_padding_x,
        text_size: typography.label,
    }
}

fn component_styles(
    colors: ColorTokens,
    typography: TypographyTokens,
    spacing: SpacingTokens,
    radii: RadiusTokens,
    borders: BorderTokens,
    density: DensityTokens,
) -> ComponentStyles {
    let primary_button = make_button_style(
        colors,
        typography,
        radii,
        borders,
        density,
        ButtonPalette {
            normal: colors.primary,
            hover: colors.primary_hover,
            pressed: colors.primary_pressed,
            foreground: Color::WHITE,
        },
    );
    let secondary_button = make_button_style(
        colors,
        typography,
        radii,
        borders,
        density,
        ButtonPalette {
            normal: colors.secondary,
            hover: colors.secondary_hover,
            pressed: colors.primary_pressed,
            foreground: colors.text,
        },
    );
    let transparent = Color::rgba(0, 0, 0, 0);
    let state = |background, foreground, border| ControlStateStyle {
        background,
        foreground,
        border,
    };
    let ghost_button = ButtonStyle {
        normal: state(transparent, colors.text, transparent),
        hovered: state(colors.secondary, colors.text, transparent),
        pressed: state(colors.secondary_hover, colors.text, transparent),
        selected: state(colors.secondary_hover, colors.text, colors.focus),
        disabled: state(transparent, colors.text_muted, transparent),
        ..secondary_button
    };
    let destructive_button = make_button_style(
        colors,
        typography,
        radii,
        borders,
        density,
        ButtonPalette {
            normal: colors.destructive,
            hover: colors.destructive_hover,
            pressed: colors.destructive,
            foreground: Color::WHITE,
        },
    );
    let icon_button = ButtonStyle {
        min_width: density.control_height,
        padding_x: spacing.sm,
        ..ghost_button
    };
    ComponentStyles {
        primary_button,
        secondary_button,
        ghost_button,
        destructive_button,
        icon_button,
    }
}

impl Theme {
    #[must_use]
    pub fn dark() -> Self {
        Self::from_colors(
            ThemeMode::Dark,
            ColorTokens {
                window: Color::rgb(20, 24, 32),
                surface: Color::rgb(28, 34, 44),
                surface_raised: Color::rgb(37, 44, 56),
                text: Color::rgb(238, 242, 248),
                text_muted: Color::rgb(164, 174, 190),
                border: Color::rgb(73, 84, 101),
                focus: Color::rgb(112, 201, 255),
                primary: Color::rgb(48, 94, 138),
                primary_hover: Color::rgb(58, 113, 166),
                primary_pressed: Color::rgb(38, 76, 113),
                secondary: Color::rgb(55, 64, 78),
                secondary_hover: Color::rgb(69, 80, 97),
                destructive: Color::rgb(157, 53, 62),
                destructive_hover: Color::rgb(190, 64, 74),
                disabled: Color::rgb(55, 61, 72),
            },
        )
    }

    #[must_use]
    pub fn light() -> Self {
        Self::from_colors(
            ThemeMode::Light,
            ColorTokens {
                window: Color::rgb(244, 247, 250),
                surface: Color::rgb(255, 255, 255),
                surface_raised: Color::rgb(250, 252, 255),
                text: Color::rgb(28, 34, 43),
                text_muted: Color::rgb(92, 103, 119),
                border: Color::rgb(196, 205, 216),
                focus: Color::rgb(0, 112, 187),
                primary: Color::rgb(32, 105, 166),
                primary_hover: Color::rgb(25, 88, 141),
                primary_pressed: Color::rgb(19, 70, 113),
                secondary: Color::rgb(225, 232, 240),
                secondary_hover: Color::rgb(208, 218, 229),
                destructive: Color::rgb(184, 47, 58),
                destructive_hover: Color::rgb(151, 35, 45),
                disabled: Color::rgb(207, 214, 223),
            },
        )
    }

    /// A dark, high-contrast palette intended for accessibility and visual testing.
    #[must_use]
    pub fn high_contrast() -> Self {
        Self::from_colors(
            ThemeMode::Dark,
            ColorTokens {
                window: Color::rgb(0, 0, 0),
                surface: Color::rgb(0, 0, 0),
                surface_raised: Color::rgb(12, 12, 12),
                text: Color::WHITE,
                text_muted: Color::rgb(214, 214, 214),
                border: Color::WHITE,
                focus: Color::rgb(255, 230, 0),
                primary: Color::rgb(0, 92, 230),
                primary_hover: Color::rgb(38, 126, 255),
                primary_pressed: Color::rgb(0, 64, 168),
                secondary: Color::rgb(30, 30, 30),
                secondary_hover: Color::rgb(64, 64, 64),
                destructive: Color::rgb(190, 0, 28),
                destructive_hover: Color::rgb(238, 30, 62),
                disabled: Color::rgb(48, 48, 48),
            },
        )
    }

    fn from_colors(mode: ThemeMode, colors: ColorTokens) -> Self {
        let typography = TypographyTokens {
            body: 16.0,
            label: 15.0,
            title: 24.0,
            caption: 13.0,
        };
        let spacing = SpacingTokens {
            xs: 4.0,
            sm: 8.0,
            md: 12.0,
            lg: 16.0,
            xl: 24.0,
        };
        let radii = RadiusTokens {
            sm: 4.0,
            md: 8.0,
            lg: 12.0,
            pill: 999.0,
        };
        let borders = BorderTokens {
            thin: 1.0,
            strong: 2.0,
        };
        let elevation = ElevationTokens {
            low_offset: 2.0,
            high_offset: 5.0,
            shadow: Color::rgba(0, 0, 0, 80),
        };
        let density = DensityTokens {
            control_height: 40.0,
            compact_control_height: 32.0,
            control_padding_x: 14.0,
        };
        let components = component_styles(colors, typography, spacing, radii, borders, density);
        Self {
            mode,
            colors,
            typography,
            spacing,
            radii,
            borders,
            elevation,
            density,
            components,
            window: colors.window,
            text: colors.text,
            button: colors.primary,
            button_hover: colors.primary_hover,
            button_pressed: colors.primary_pressed,
            button_disabled: colors.disabled,
            focus: colors.focus,
        }
    }

    #[must_use]
    pub const fn button_style(&self, variant: ButtonVariant) -> ButtonStyle {
        match variant {
            ButtonVariant::Primary => self.components.primary_button,
            ButtonVariant::Secondary => self.components.secondary_button,
            ButtonVariant::Ghost => self.components.ghost_button,
            ButtonVariant::Destructive => self.components.destructive_button,
            ButtonVariant::Icon => self.components.icon_button,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}
