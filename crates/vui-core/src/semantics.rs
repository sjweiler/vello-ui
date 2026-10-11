use crate::{Rect, WidgetId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticRole {
    Window,
    Group,
    Label,
    Image,
    Toolbar,
    Navigation,
    StatusBar,
    Tab,
    Dialog,
    Button,
    Checkbox,
    Menu,
    MenuItem,
    MenuItemCheckbox,
    MenuItemRadio,
    List,
    ListItem,
    Tree,
    TreeItem,
    Table,
    TableRow,
    ColumnHeader,
    Cell,
    TextInput,
    MultilineTextInput,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticNode {
    pub id: WidgetId,
    pub role: SemanticRole,
    pub bounds: Rect,
    pub label: String,
    pub disabled: bool,
    pub focused: bool,
    pub checked: Option<bool>,
    pub value: Option<String>,
    pub ime_cursor_area: Option<Rect>,
}
