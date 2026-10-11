use vui_core::{
    Button, ButtonVariant, Checkbox, Column, Command, CommandButton, CommandDescriptor,
    CommandHost, CommandIconButton, CommandPalette, CommandScope, ContextMenu, DesktopApp,
    DesktopNotification, DesktopSender, Dialog, Flex, FlexChild, IconButton, IconName, Insets, Key,
    KeyChord, Keyed, Label, ListItem, ListView, Menu, MenuButton, MenuItem, NativeMenu,
    NativeMenuItem, NavigationItem, OverlayHost, Padding, ProgressBar, RasterImage, Responsive,
    ScrollView, Sidebar, Size, SizedBox, SortDirection, Spacer, SplitPane, StatusBar, Surface, Tab,
    TabBar, TableAlignment, TableColumn, TableView, TextArea, TextField, Theme, ThemeScope,
    Toolbar, TrayDescriptor, TreeNode, TreeView, Widget, WidgetKey, WindowCommand,
    WindowDescriptor,
};

struct Settings {
    animations: bool,
    dark_theme: bool,
    display_name: String,
    applied: u32,
    active_tab: usize,
}

enum Message {
    AnimationsChanged(bool),
    ThemeChanged(bool),
    DisplayNameChanged(String),
    Apply,
    SelectTab(usize),
}

fn application_commands(model: &Settings, sender: &DesktopSender<Message>) -> Vec<Command> {
    let general = sender.clone();
    let appearance = sender.clone();
    let keyboard = sender.clone();
    let theme = sender.clone();
    let apply = sender.clone();
    let active_tab = model.active_tab;
    let dark_theme = model.dark_theme;
    vec![
        Command::action("view.general", "General", move || {
            general.emit(Message::SelectTab(0));
        })
        .description("Open general settings")
        .keywords("settings navigation")
        .icon(IconName::Settings)
        .shortcut(KeyChord::primary(Key::Character("1".to_owned())))
        .scope(CommandScope::Window)
        .selected(active_tab == 0),
        Command::action("view.appearance", "Appearance", move || {
            appearance.emit(Message::SelectTab(1));
        })
        .description("Open appearance settings")
        .keywords("theme navigation")
        .icon(IconName::Command)
        .shortcut(KeyChord::primary(Key::Character("2".to_owned())))
        .scope(CommandScope::Window)
        .selected(active_tab == 1),
        Command::action("view.keyboard", "Keyboard", move || {
            keyboard.emit(Message::SelectTab(2));
        })
        .description("Open keyboard settings")
        .keywords("shortcuts navigation")
        .icon(IconName::Menu)
        .shortcut(KeyChord::primary(Key::Character("3".to_owned())))
        .scope(CommandScope::Window)
        .selected(active_tab == 2),
        Command::action("theme.toggle", "Use dark theme", move || {
            theme.emit(Message::ThemeChanged(!dark_theme));
        })
        .description("Switch between light and dark themes")
        .keywords("appearance light dark")
        .icon(IconName::Settings)
        .shortcut(KeyChord::primary(Key::Character("d".to_owned())))
        .scope(CommandScope::Window)
        .checked(dark_theme),
        Command::action("settings.apply", "Apply changes", move || {
            apply.emit(Message::Apply);
        })
        .description("Apply the current settings")
        .keywords("save confirm")
        .icon(IconName::Command)
        .shortcut(KeyChord::primary(Key::Character("s".to_owned())))
        .scope(CommandScope::Window),
    ]
}

fn descriptor(commands: &[CommandDescriptor], id: &str) -> CommandDescriptor {
    commands
        .iter()
        .find(|command| command.id.as_str() == id)
        .cloned()
        .expect("application command must be registered")
}

fn collection_examples() -> impl Widget {
    let tree = TreeView::new([
        TreeNode::branch(
            "workspace",
            "workspace",
            [
                TreeNode::branch(
                    "src",
                    "src",
                    [
                        TreeNode::leaf("src/lib.rs", "lib.rs"),
                        TreeNode::leaf("src/main.rs", "main.rs"),
                    ],
                ),
                TreeNode::unloaded("remote", "remote"),
                TreeNode::loading("packages", "packages"),
                TreeNode::error("cache", "cache", "offline"),
            ],
        ),
        TreeNode::leaf("readme", "README.md"),
    ])
    .expanded([WidgetKey::new("workspace"), WidgetKey::new("src")])
    .row_height(28.0)
    .viewport_height(190.0)
    .draggable(true)
    .on_drop(|drop| {
        println!(
            "tree move {} {:?} {}",
            drop.dragged.as_str(),
            drop.position,
            drop.target.as_str()
        );
    })
    .on_activate(|key| println!("opened {}", key.as_str()));
    let table = TableView::new(
        10_000,
        [
            TableColumn::new("name", "Name", 210.0),
            TableColumn::new("kind", "Kind", 100.0),
            TableColumn::new("size", "Size", 100.0).alignment(TableAlignment::Right),
            TableColumn::new("modified", "Modified", 150.0),
        ],
        |row, column| match column.as_str() {
            "name" => format!("document-{row:05}.txt"),
            "kind" => "Text document".to_owned(),
            "size" => format!("{} KB", 4 + row % 4096),
            "modified" => format!("2026-10-{:02}", 1 + row % 28),
            _ => String::new(),
        },
    )
    .row_keys(|row| WidgetKey::new(format!("document-{row}")))
    .sort("name", SortDirection::Ascending)
    .row_height(28.0)
    .viewport_height(220.0)
    .draggable(true)
    .on_drop(|drop| {
        println!(
            "table move {} {:?} {}",
            drop.dragged.as_str(),
            drop.position,
            drop.target.as_str()
        );
    })
    .on_sort_change(|column, direction| {
        println!("sort {}: {direction:?}", column.as_str());
    })
    .on_activate(|key| println!("opened table row {}", key.as_str()));
    let list = ListView::new(10_000, |index| {
        let name = format!("Workspace item {index:05}");
        ListItem::new(format!("workspace-{index}"), name.clone(), Label::new(name))
    })
    .row_height(30.0)
    .viewport_height(180.0)
    .draggable(true)
    .on_drop(|drop| {
        println!(
            "list move {} {:?} {}",
            drop.dragged.as_str(),
            drop.position,
            drop.target.as_str()
        );
    })
    .on_activate(|key| println!("activated {}", key.as_str()));
    Column::new([
        Box::new(ProgressBar::new(0.65).label("Background indexing")) as Box<dyn Widget>,
        Box::new(tree) as Box<dyn Widget>,
        Box::new(table) as Box<dyn Widget>,
        Box::new(list) as Box<dyn Widget>,
    ])
}

fn settings_content(
    model: &Settings,
    sender: &DesktopSender<Message>,
    commands: &[CommandDescriptor],
) -> impl Widget + use<> {
    let animation_sender = sender.clone();
    let theme_sender = sender.clone();
    let name_sender = sender.clone();
    let menu_commands = commands.to_vec();
    ScrollView::new(Padding::new(
        Insets::all(24.0),
        Column::new([
            Box::new(Label::new("VUI settings — مرحبا").text_size(24.0)) as Box<dyn Widget>,
            Box::new(Label::new(format!(
                "Applied {} time(s) — display name: {}",
                model.applied,
                if model.display_name.is_empty() {
                    "not set"
                } else {
                    &model.display_name
                }
            ))) as Box<dyn Widget>,
            Box::new(Checkbox::new(
                "Enable animations",
                model.animations,
                move |checked| animation_sender.emit(Message::AnimationsChanged(checked)),
            )) as Box<dyn Widget>,
            Box::new(Checkbox::new(
                "Use dark theme",
                model.dark_theme,
                move |checked| theme_sender.emit(Message::ThemeChanged(checked)),
            )) as Box<dyn Widget>,
            Box::new(Keyed::new(
                "display-name",
                TextField::new(model.display_name.clone(), move |value| {
                    name_sender.emit(Message::DisplayNameChanged(value.to_owned()));
                })
                .placeholder("Display name"),
            )) as Box<dyn Widget>,
            Box::new(
                TextArea::new("", |value| println!("notes: {value:?}"))
                    .placeholder("Notes")
                    .min_lines(3),
            ) as Box<dyn Widget>,
            Box::new(
                Flex::row([
                    FlexChild::fixed(CommandButton::new(descriptor(commands, "settings.apply"))),
                    FlexChild::flexible(1.0, Spacer::new(0.0, 0.0)),
                    FlexChild::fixed(
                        Button::new("Unavailable", || {})
                            .variant(ButtonVariant::Secondary)
                            .disabled(true),
                    ),
                ])
                .gap(10.0),
            ) as Box<dyn Widget>,
            Box::new(MenuButton::new("Open command menu", move || {
                Menu::new([
                    MenuItem::command(descriptor(&menu_commands, "theme.toggle")),
                    MenuItem::separator(),
                    MenuItem::command(descriptor(&menu_commands, "settings.apply")),
                ])
            })) as Box<dyn Widget>,
            Box::new(collection_examples()) as Box<dyn Widget>,
        ])
        .gap(16.0),
    ))
}

fn sidebar(commands: &[CommandDescriptor]) -> impl Widget + use<> {
    Sidebar::new([
        Box::new(Label::new("VUI").text_size(28.0)) as Box<dyn Widget>,
        Box::new(NavigationItem::command(descriptor(
            commands,
            "view.general",
        ))) as Box<dyn Widget>,
        Box::new(NavigationItem::command(descriptor(
            commands,
            "view.appearance",
        ))) as Box<dyn Widget>,
        Box::new(NavigationItem::command(descriptor(
            commands,
            "view.keyboard",
        ))) as Box<dyn Widget>,
    ])
}

fn inspector_button(desktop: DesktopApp<Settings, Message>) -> Button {
    Button::action("Inspector", move |_, ctx| {
        let inspector = desktop.window(|model, _sender| {
            Padding::new(
                Insets::all(24.0),
                Column::new([
                    Box::new(Label::new("Detached inspector").text_size(22.0)) as Box<dyn Widget>,
                    Box::new(Label::new(format!(
                        "Display name: {}",
                        if model.display_name.is_empty() {
                            "not set"
                        } else {
                            &model.display_name
                        }
                    ))) as Box<dyn Widget>,
                    Box::new(Label::new(format!("Applied: {}", model.applied))) as Box<dyn Widget>,
                    Box::new(Label::new(
                        "This view rebuilds from the same model as the workspace.",
                    )) as Box<dyn Widget>,
                    Box::new(Button::action("Close inspector", |_, ctx| {
                        if let Some(id) = ctx.current_window() {
                            ctx.window_command(WindowCommand::Close(id));
                        }
                    })) as Box<dyn Widget>,
                ])
                .gap(16.0),
            )
        });
        ctx.open_window(
            WindowDescriptor::new("VUI inspector", Size::new(420.0, 280.0))
                .min_size(Size::new(300.0, 180.0))
                .key("inspector")
                .singleton(true),
            inspector,
        );
    })
}

fn notification_button() -> Button {
    Button::action("Notify", |_, ctx| {
        ctx.notify(
            DesktopNotification::new(
                "showcase-applied",
                "VUI desktop services",
                "Typed notifications can return action events to the widget tree.",
            )
            .action("open", "Open VUI"),
        );
    })
}

fn application_shell(
    model: &Settings,
    sender: &DesktopSender<Message>,
    commands: &[CommandDescriptor],
    desktop: DesktopApp<Settings, Message>,
) -> impl Widget + use<> {
    let tab_sender = sender.clone();
    let tabs = SizedBox::new(
        TabBar::new(
            [
                Tab::new("Settings"),
                Tab::new("Preview").closable(true),
                Tab::new("Logs").closable(true),
            ],
            move |index| tab_sender.emit(Message::SelectTab(index)),
        )
        .active(model.active_tab)
        .on_close(|index| println!("close tab {index}"))
        .reorderable(true)
        .on_reorder(|from, to| println!("reorder tab {from} -> {to}")),
    )
    .fill_width();
    let palette_commands = commands.to_vec();
    let toolbar = SizedBox::new(Toolbar::new([
        Box::new(IconButton::new(IconName::Menu, "Toggle sidebar", || {}).tooltip("Toggle sidebar"))
            as Box<dyn Widget>,
        Box::new(
            IconButton::action(IconName::Command, "Command palette", move |_, ctx| {
                ctx.show_modal(CommandPalette::new(palette_commands.clone()));
            })
            .tooltip("Command palette"),
        ) as Box<dyn Widget>,
        Box::new(CommandIconButton::new(descriptor(commands, "theme.toggle"))) as Box<dyn Widget>,
        Box::new(
            IconButton::action(IconName::Settings, "Confirm changes", |_, ctx| {
                ctx.show_modal(Dialog::confirmation(
                    "Apply changes?",
                    "This demonstrates a modal shell dialog.",
                    "Apply",
                    || println!("confirmed"),
                ));
            })
            .tooltip("Open dialog"),
        ) as Box<dyn Widget>,
        Box::new(inspector_button(desktop)) as Box<dyn Widget>,
        Box::new(notification_button()) as Box<dyn Widget>,
        Box::new(Button::action("Quit", |_, ctx| {
            ctx.window_command(WindowCommand::QuitApplication);
        })) as Box<dyn Widget>,
        Box::new(Button::action("Window modal", |_, ctx| {
            ctx.open_window(
                WindowDescriptor::new("Confirm", Size::new(360.0, 190.0)).modal(true),
                Padding::new(
                    Insets::all(24.0),
                    Column::new([
                        Box::new(Label::new("A separate modal window").text_size(20.0))
                            as Box<dyn Widget>,
                        Box::new(Label::new(
                            "Its parent is inferred from the current runtime.",
                        )) as Box<dyn Widget>,
                        Box::new(Button::action("Done", |_, ctx| {
                            if let Some(id) = ctx.current_window() {
                                ctx.window_command(WindowCommand::Close(id));
                            }
                        })) as Box<dyn Widget>,
                    ])
                    .gap(14.0),
                ),
            );
        })) as Box<dyn Widget>,
    ]))
    .fill_width();
    let status = SizedBox::new(StatusBar::new([
        Box::new(Label::new("Ready").text_size(13.0)) as Box<dyn Widget>,
        Box::new(Label::new("UTF-8").text_size(13.0)) as Box<dyn Widget>,
    ]))
    .fill_width();
    Flex::column([
        FlexChild::fixed(toolbar),
        FlexChild::fixed(tabs),
        FlexChild::flexible(
            1.0,
            SizedBox::new(settings_content(model, sender, commands))
                .fill_width()
                .fill_height(),
        ),
        FlexChild::fixed(status),
    ])
}

fn tray_icon() -> RasterImage {
    let mut pixels = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16 {
        for x in 0..16 {
            let inside = (3..13).contains(&x) && (3..13).contains(&y);
            pixels.extend_from_slice(if inside {
                &[0x7c, 0x5c, 0xfc, 0xff]
            } else {
                &[0x00, 0x00, 0x00, 0x00]
            });
        }
    }
    RasterImage::new(16, 16, pixels).expect("fixed-size tray icon")
}

fn showcase_tray() -> TrayDescriptor {
    let command = |id: &'static str, label: &'static str| {
        Command::action(id, label, || {}).descriptor().clone()
    };
    let menu = NativeMenu::new([
        NativeMenuItem::submenu(
            "Navigate",
            NativeMenu::commands([
                command("view.general", "General"),
                command("view.appearance", "Appearance"),
                command("view.keyboard", "Keyboard"),
            ]),
        ),
        NativeMenuItem::Separator,
        NativeMenuItem::Command(command("settings.apply", "Apply changes")),
    ]);
    TrayDescriptor::new("showcase", "VUI showcase", menu).icon(tray_icon())
}

fn interface() -> impl Widget {
    let desktop = DesktopApp::new(
        Settings {
            animations: true,
            dark_theme: true,
            display_name: String::new(),
            applied: 0,
            active_tab: 0,
        },
        |model, message| match message {
            Message::AnimationsChanged(enabled) => model.animations = enabled,
            Message::ThemeChanged(dark) => model.dark_theme = dark,
            Message::DisplayNameChanged(name) => model.display_name = name,
            Message::Apply => model.applied += 1,
            Message::SelectTab(index) => model.active_tab = index,
        },
    );
    let view_desktop = desktop.clone();
    desktop.window(move |model, sender| {
        let commands = application_commands(model, &sender);
        let descriptors = commands
            .iter()
            .map(|command| command.descriptor().clone())
            .collect::<Vec<_>>();
        let compact = SizedBox::new(application_shell(
            model,
            &sender,
            &descriptors,
            view_desktop.clone(),
        ))
        .fill_width()
        .fill_height();
        let wide = SplitPane::horizontal(
            sidebar(&descriptors),
            application_shell(model, &sender, &descriptors, view_desktop.clone()),
        )
        .ratio(0.24)
        .min_pane(160.0);
        let workspace = Responsive::new(700.0, compact, wide);
        let context_commands = descriptors.clone();
        let overlays = OverlayHost::new(ContextMenu::new(workspace, move || {
            Menu::new([
                MenuItem::command(descriptor(&context_commands, "theme.toggle")),
                MenuItem::command(descriptor(&context_commands, "settings.apply")),
            ])
        }));
        let command_host = CommandHost::new(commands, overlays);
        let shell = SizedBox::new(Surface::new(command_host))
            .fill_width()
            .fill_height();
        ThemeScope::new(
            if model.dark_theme {
                Theme::dark()
            } else {
                Theme::light()
            },
            shell,
        )
    })
}

fn main() -> anyhow::Result<()> {
    let session_path = std::env::var_os("XDG_STATE_HOME")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join("vui/hello-session.json");
    let application = vui_winit::DesktopApplication::new("dev.vui.showcase", interface())
        .title("VUI hello")
        .session_path(session_path)
        .single_instance(true)
        .tray(showcase_tray())
        .close_to_tray(true);
    #[cfg(target_os = "linux")]
    let application =
        application.platform_services(vui_platform_linux::LinuxPlatformServices::new()?);
    application.run()
}
