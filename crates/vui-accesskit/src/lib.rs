//! Converts VUI's platform-neutral semantics into AccessKit tree updates.

use accesskit::{
    Action, ActionRequest, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate,
};
use vui_core::{Event, SemanticAction, SemanticNode, SemanticRole, WidgetId};

const ROOT: NodeId = NodeId(0);

#[must_use]
pub fn tree_update(nodes: &[SemanticNode]) -> TreeUpdate {
    let children: Vec<_> = nodes.iter().map(node_id).collect();
    let mut root = Node::new(Role::Window);
    root.set_children(children);

    let mut update_nodes = Vec::with_capacity(nodes.len() + 1);
    update_nodes.push((ROOT, root));
    update_nodes.extend(nodes.iter().map(|semantic| {
        let mut node = Node::new(role(semantic.role));
        if semantic.role == SemanticRole::Label {
            node.set_value(semantic.label.clone());
        } else {
            node.set_label(semantic.label.clone());
        }
        if let Some(value) = &semantic.value {
            node.set_value(value.clone());
        }
        node.set_bounds(Rect {
            x0: semantic.bounds.x,
            y0: semantic.bounds.y,
            x1: semantic.bounds.x + semantic.bounds.width,
            y1: semantic.bounds.y + semantic.bounds.height,
        });
        if semantic.disabled {
            node.set_disabled();
        }
        if matches!(
            semantic.role,
            SemanticRole::Button
                | SemanticRole::Tab
                | SemanticRole::Checkbox
                | SemanticRole::Switch
                | SemanticRole::RadioButton
                | SemanticRole::Slider
                | SemanticRole::ComboBox
                | SemanticRole::MenuItem
                | SemanticRole::MenuItemCheckbox
                | SemanticRole::MenuItemRadio
                | SemanticRole::List
                | SemanticRole::Tree
                | SemanticRole::Table
                | SemanticRole::TextInput
                | SemanticRole::PasswordInput
                | SemanticRole::MultilineTextInput
        ) {
            node.add_action(Action::Focus);
        }
        if matches!(
            semantic.role,
            SemanticRole::Button
                | SemanticRole::Tab
                | SemanticRole::Checkbox
                | SemanticRole::Switch
                | SemanticRole::RadioButton
                | SemanticRole::ComboBox
                | SemanticRole::MenuItem
                | SemanticRole::MenuItemCheckbox
                | SemanticRole::MenuItemRadio
                | SemanticRole::ListItem
                | SemanticRole::TreeItem
                | SemanticRole::TableRow
                | SemanticRole::ColumnHeader
                | SemanticRole::Cell
        ) {
            node.add_action(Action::Click);
        }
        if semantic.role == SemanticRole::Slider {
            node.add_action(Action::Increment);
            node.add_action(Action::Decrement);
        }
        if let Some(checked) = semantic.checked {
            node.set_toggled(Toggled::from(checked));
        }
        (node_id(semantic), node)
    }));

    let focus = nodes.iter().find(|node| node.focused).map_or(ROOT, node_id);
    let mut tree = TreeInfo::new(ROOT);
    tree.toolkit_name = Some("VUI".to_owned());
    tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());
    TreeUpdate {
        nodes: update_nodes,
        tree: Some(tree),
        tree_id: TreeId::ROOT,
        focus,
    }
}

#[must_use]
pub fn event_for_action(request: &ActionRequest) -> Option<Event> {
    let action = match request.action {
        Action::Click => SemanticAction::Click,
        Action::Focus => SemanticAction::Focus,
        Action::Increment => SemanticAction::Increment,
        Action::Decrement => SemanticAction::Decrement,
        _ => return None,
    };
    Some(Event::AccessibilityAction {
        target: WidgetId::from_raw(request.target_node.0),
        action,
    })
}

fn node_id(node: &SemanticNode) -> NodeId {
    NodeId(node.id.raw())
}

fn role(role: SemanticRole) -> Role {
    match role {
        SemanticRole::Window => Role::Window,
        SemanticRole::Group
        | SemanticRole::Toolbar
        | SemanticRole::Navigation
        | SemanticRole::StatusBar
        | SemanticRole::Dialog => Role::Group,
        SemanticRole::Label => Role::Label,
        SemanticRole::Image => Role::Image,
        SemanticRole::Tab => Role::Tab,
        SemanticRole::Button => Role::Button,
        SemanticRole::Checkbox => Role::CheckBox,
        SemanticRole::Switch => Role::Switch,
        SemanticRole::RadioButton => Role::RadioButton,
        SemanticRole::Slider => Role::Slider,
        SemanticRole::ComboBox => Role::ComboBox,
        SemanticRole::ProgressIndicator => Role::ProgressIndicator,
        SemanticRole::Alert => Role::Alert,
        SemanticRole::Menu => Role::Menu,
        SemanticRole::MenuItem => Role::MenuItem,
        SemanticRole::MenuItemCheckbox => Role::MenuItemCheckBox,
        SemanticRole::MenuItemRadio => Role::MenuItemRadio,
        SemanticRole::List => Role::List,
        SemanticRole::ListItem => Role::ListItem,
        SemanticRole::Tree => Role::Tree,
        SemanticRole::TreeItem => Role::TreeItem,
        SemanticRole::Table => Role::Table,
        SemanticRole::TableRow => Role::Row,
        SemanticRole::ColumnHeader => Role::ColumnHeader,
        SemanticRole::Cell => Role::Cell,
        SemanticRole::TextInput => Role::TextInput,
        SemanticRole::PasswordInput => Role::PasswordInput,
        SemanticRole::MultilineTextInput => Role::MultilineTextInput,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vui_core::{Rect as UiRect, WidgetId};

    #[test]
    fn converts_focus_and_checked_state() {
        let semantics = vec![SemanticNode {
            id: WidgetId::next(),
            role: SemanticRole::Checkbox,
            bounds: UiRect::new(10.0, 20.0, 100.0, 24.0),
            label: "Animations".to_owned(),
            disabled: false,
            focused: true,
            checked: Some(true),
            value: None,
            ime_cursor_area: None,
        }];
        let update = tree_update(&semantics);
        assert_eq!(update.nodes.len(), 2);
        assert_eq!(update.focus, NodeId(semantics[0].id.raw()));
        assert_eq!(update.nodes[1].1.toggled(), Some(Toggled::True));
        assert!(update.nodes[1].1.supports_action(Action::Click));
    }

    #[test]
    fn converts_supported_action_requests() {
        let target = WidgetId::next();
        let request = ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: NodeId(target.raw()),
            data: None,
        };
        assert_eq!(
            event_for_action(&request),
            Some(Event::AccessibilityAction {
                target,
                action: SemanticAction::Focus,
            })
        );
    }

    #[test]
    fn converts_menu_item_roles_and_actions() {
        let semantics = vec![
            SemanticNode {
                id: WidgetId::next(),
                role: SemanticRole::Menu,
                bounds: UiRect::new(0.0, 0.0, 160.0, 80.0),
                label: String::new(),
                disabled: false,
                focused: false,
                checked: None,
                value: None,
                ime_cursor_area: None,
            },
            SemanticNode {
                id: WidgetId::next(),
                role: SemanticRole::MenuItemCheckbox,
                bounds: UiRect::new(6.0, 6.0, 148.0, 34.0),
                label: "Show panel".to_owned(),
                disabled: false,
                focused: true,
                checked: Some(true),
                value: None,
                ime_cursor_area: None,
            },
        ];
        let update = tree_update(&semantics);
        assert_eq!(update.nodes[1].1.role(), Role::Menu);
        assert_eq!(update.nodes[2].1.role(), Role::MenuItemCheckBox);
        assert!(update.nodes[2].1.supports_action(Action::Focus));
        assert!(update.nodes[2].1.supports_action(Action::Click));
        assert_eq!(update.nodes[2].1.toggled(), Some(Toggled::True));
    }
}
