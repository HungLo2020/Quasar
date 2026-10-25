use crate::math::{Edges, Rect, Vec2};
use crate::theme::Theme;
use crate::widget::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiError {
    UnknownNode,
    InvalidValue,
}

impl std::fmt::Display for UiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UnknownNode => "unknown UI node",
            Self::InvalidValue => "invalid UI value",
        })
    }
}
impl std::error::Error for UiError {}
pub type UiResult<T> = Result<T, UiError>;

#[derive(Clone, Debug)]
pub struct UiNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub widget: Widget,
    pub item: LayoutItem,
    pub layer: Layer,
    pub visible: bool,
    pub enabled: bool,
    pub modal: bool,
    pub state: WidgetState,
    pub scroll: Vec2,
    pub content_size: Vec2,
    /// Horizontal text offset for a text field, maintained by `Ui::update`.
    pub text_scroll: f32,
    /// Latest IME preedit for a focused text field.
    pub preedit: String,
    pub preedit_cursor: usize,
}

impl UiNode {
    fn new(
        id: NodeId,
        parent: Option<NodeId>,
        widget: Widget,
        item: LayoutItem,
        layer: Layer,
        modal: bool,
    ) -> Self {
        Self {
            id,
            parent,
            children: Vec::new(),
            widget,
            item,
            layer,
            visible: true,
            enabled: true,
            modal,
            state: WidgetState::default(),
            scroll: Vec2::ZERO,
            content_size: Vec2::ZERO,
            text_scroll: 0.0,
            preedit: String::new(),
            preedit_cursor: 0,
        }
    }
}

pub struct Ui {
    nodes: BTreeMap<NodeId, UiNode>,
    roots: Vec<NodeId>,
    next_id: u64,
    focus: Option<NodeId>,
    focus_initialized: bool,
    modal: Option<NodeId>,
    pointer_capture: Option<NodeId>,
    slider_drag: Option<NodeId>,
    text_drag: Option<NodeId>,
    capturing: Option<(NodeId, String)>,
    strings: BTreeMap<String, String>,
    theme: Theme,
    layout: Vec<LayoutEntry>,
    content_sizes: BTreeMap<NodeId, Vec2>,
    focus_order: Vec<NodeId>,
    scale: f32,
}

impl Ui {
    pub fn new(theme: Theme) -> Self {
        Self {
            nodes: BTreeMap::new(),
            roots: Vec::new(),
            next_id: 1,
            focus: None,
            focus_initialized: false,
            modal: None,
            pointer_capture: None,
            slider_drag: None,
            text_drag: None,
            capturing: None,
            strings: BTreeMap::new(),
            theme,
            layout: Vec::new(),
            content_sizes: BTreeMap::new(),
            focus_order: Vec::new(),
            scale: 1.0,
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }
    pub fn scale(&self) -> f32 {
        self.scale
    }
    pub fn node(&self, id: NodeId) -> Option<&UiNode> {
        self.nodes.get(&id)
    }
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }
    pub fn layout_entries(&self) -> &[LayoutEntry] {
        &self.layout
    }
    pub fn focus_order(&self) -> &[NodeId] {
        &self.focus_order
    }
    pub fn focused(&self) -> Option<NodeId> {
        self.focus
    }
    pub fn modal(&self) -> Option<NodeId> {
        self.modal
    }
    pub fn is_capturing(&self) -> bool {
        self.capturing.is_some()
    }
    pub fn set_localized(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.strings.insert(key.into(), value.into());
    }
    pub fn resolve_text<'a>(&'a self, text: &'a UiText) -> &'a str {
        text.key
            .as_ref()
            .and_then(|key| self.strings.get(key))
            .map_or(text.fallback.as_str(), String::as_str)
    }

    pub fn add_root(
        &mut self,
        widget: Widget,
        item: LayoutItem,
        layer: Layer,
        modal: bool,
    ) -> NodeId {
        let id = self.allocate_id();
        let node = UiNode::new(id, None, widget, item, layer, modal);
        self.nodes.insert(id, node);
        self.roots.push(id);
        id
    }

    pub fn add_child(
        &mut self,
        parent: NodeId,
        widget: Widget,
        item: LayoutItem,
    ) -> UiResult<NodeId> {
        let layer = self.nodes.get(&parent).ok_or(UiError::UnknownNode)?.layer;
        let id = self.allocate_id();
        let node = UiNode::new(id, Some(parent), widget, item, layer, false);
        self.nodes.insert(id, node);
        self.nodes
            .get_mut(&parent)
            .ok_or(UiError::UnknownNode)?
            .children
            .push(id);
        Ok(id)
    }

    fn allocate_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn set_visible(&mut self, id: NodeId, visible: bool) -> UiResult<()> {
        self.nodes.get_mut(&id).ok_or(UiError::UnknownNode)?.visible = visible;
        if !visible && self.focus == Some(id) {
            self.focus = None;
        }
        Ok(())
    }
    pub fn set_enabled(&mut self, id: NodeId, enabled: bool) -> UiResult<()> {
        self.nodes.get_mut(&id).ok_or(UiError::UnknownNode)?.enabled = enabled;
        Ok(())
    }
    pub fn set_text(&mut self, id: NodeId, text: UiText) -> UiResult<()> {
        match &mut self.nodes.get_mut(&id).ok_or(UiError::UnknownNode)?.widget {
            Widget::Label { text: value, .. }
            | Widget::Button { text: value, .. }
            | Widget::Slider { label: value, .. }
            | Widget::Checkbox { label: value, .. }
            | Widget::Select { label: value, .. }
            | Widget::Progress { label: value, .. }
            | Widget::Binding { label: value, .. } => *value = text,
            Widget::TextInput { placeholder, .. } => *placeholder = text,
            _ => return Err(UiError::InvalidValue),
        }
        Ok(())
    }
    pub fn set_value(&mut self, id: NodeId, value: f32) -> UiResult<f32> {
        let node = self.nodes.get_mut(&id).ok_or(UiError::UnknownNode)?;
        match &mut node.widget {
            Widget::Slider {
                value: current,
                min,
                max,
                step,
                ..
            } => {
                if !value.is_finite() || !min.is_finite() || !max.is_finite() || *max < *min {
                    return Err(UiError::InvalidValue);
                }
                let value = value.clamp(*min, *max);
                let value = if *step > 0.0 {
                    (*min + ((value - *min) / *step).round() * *step).clamp(*min, *max)
                } else {
                    value
                };
                *current = value;
                Ok(value)
            }
            Widget::Progress {
                value: current,
                max,
                ..
            } => {
                if !value.is_finite() || !max.is_finite() || *max <= 0.0 {
                    return Err(UiError::InvalidValue);
                }
                *current = value.clamp(0.0, *max);
                Ok(*current)
            }
            _ => Err(UiError::InvalidValue),
        }
    }
    pub fn set_checked(&mut self, id: NodeId, checked: bool) -> UiResult<()> {
        match &mut self.nodes.get_mut(&id).ok_or(UiError::UnknownNode)?.widget {
            Widget::Checkbox { checked: value, .. } => {
                *value = checked;
                Ok(())
            }
            _ => Err(UiError::InvalidValue),
        }
    }
    pub fn set_selected(&mut self, id: NodeId, selected: usize) -> UiResult<()> {
        let node = self.nodes.get_mut(&id).ok_or(UiError::UnknownNode)?;
        match &mut node.widget {
            Widget::Select {
                options,
                selected: value,
                ..
            }
            | Widget::Tabs {
                tabs: options,
                selected: value,
                ..
            } => {
                if options.is_empty() {
                    return Err(UiError::InvalidValue);
                }
                *value = selected.min(options.len() - 1);
                Ok(())
            }
            Widget::List {
                items,
                selected: value,
                ..
            } => {
                if items.is_empty() {
                    return Err(UiError::InvalidValue);
                }
                *value = selected.min(items.len() - 1);
                Ok(())
            }
            _ => Err(UiError::InvalidValue),
        }
    }
    pub fn set_scroll(&mut self, id: NodeId, offset: Vec2) -> UiResult<()> {
        let content = self.content_sizes.get(&id).copied().unwrap_or(Vec2::ZERO);
        let viewport = self
            .layout
            .iter()
            .find(|entry| entry.id == id)
            .map_or(Vec2::ZERO, |entry| {
                Vec2::new(entry.rect.width, entry.rect.height)
            });
        let node = self.nodes.get_mut(&id).ok_or(UiError::UnknownNode)?;
        node.scroll = Vec2::new(
            offset.x.clamp(0.0, (content.x - viewport.x).max(0.0)),
            offset.y.clamp(0.0, (content.y - viewport.y).max(0.0)),
        );
        Ok(())
    }
    pub fn set_modal(&mut self, modal: Option<NodeId>) {
        self.modal = modal;
        self.focus_initialized = true;
        if let Some(id) = modal {
            self.focus = self.first_focusable_in(id);
        }
    }
    pub fn focus(&mut self, id: Option<NodeId>) {
        self.focus_initialized = true;
        self.focus = id.filter(|id| {
            self.nodes
                .get(id)
                .is_some_and(|node| node.visible && node.enabled && node.widget.focusable())
        });
    }
    pub fn begin_rebind(&mut self, id: NodeId) -> UiResult<()> {
        let node = self.nodes.get(&id).ok_or(UiError::UnknownNode)?;
        let Widget::Binding { action, .. } = &node.widget else {
            return Err(UiError::InvalidValue);
        };
        self.capturing = Some((id, action.clone()));
        Ok(())
    }
    pub fn finish_rebind(&mut self, value: impl Into<String>) -> Option<UiEvent> {
        let (id, action) = self.capturing.take()?;
        let value = value.into();
        match &mut self.nodes.get_mut(&id)?.widget {
            Widget::Binding { value: current, .. } => *current = value.clone(),
            _ => return None,
        }
        Some(UiEvent::RebindCaptured { id, action, value })
    }
    pub fn cancel_rebind(&mut self) {
        self.capturing = None;
    }

    pub fn layout(
        &mut self,
        viewport: Rect,
        scale: f32,
        safe_area: Edges,
        measure: &dyn TextMeasure,
    ) {
        if !viewport.valid()
            || !scale.is_finite()
            || scale <= 0.0
            || !safe_area.scaled(scale).left.is_finite()
        {
            self.layout.clear();
            return;
        }
        self.scale = scale;
        let safe = Edges {
            left: safe_area.left * scale,
            top: safe_area.top * scale,
            right: safe_area.right * scale,
            bottom: safe_area.bottom * scale,
        };
        let root_area = viewport.inset(safe);
        let (entries, content_sizes) = {
            let mut pass = LayoutPass {
                ui: self,
                measure,
                scale,
                entries: Vec::new(),
                content_sizes: BTreeMap::new(),
            };
            let roots = self.roots.clone();
            for root in roots {
                if let Some(node) = pass.ui.nodes.get(&root) {
                    if !node.visible {
                        continue;
                    }
                } else {
                    continue;
                }
                let size = pass.resolve_root_size(root, root_area);
                let rect = Rect::new(root_area.x, root_area.y, size.x, size.y);
                pass.layout_node(root, rect, root_area, root_area);
            }
            (pass.entries, pass.content_sizes)
        };
        self.layout = entries;
        self.content_sizes = content_sizes;
        self.focus_order = self
            .layout
            .iter()
            .filter_map(|entry| {
                self.nodes.get(&entry.id).and_then(|node| {
                    (node.visible && node.enabled && node.widget.focusable()).then_some(entry.id)
                })
            })
            .collect();
        if let Some(modal) = self.modal {
            if self
                .focus
                .is_none_or(|focus| !self.is_descendant_or_self(focus, modal))
            {
                self.focus = self.first_focusable_in(modal);
            }
        } else if !self.focus_initialized {
            self.focus = self.focus_order.first().copied();
            self.focus_initialized = true;
        }
    }

    #[allow(clippy::collapsible_if)]
    pub fn update(
        &mut self,
        input: &UiInput,
        viewport: Rect,
        scale: f32,
        safe_area: Edges,
        measure: &dyn TextMeasure,
        dt: f32,
    ) -> UiFrame {
        self.layout(viewport, scale, safe_area, measure);
        let mut frame = UiFrame {
            focus: self.focus,
            ..UiFrame::default()
        };
        self.animate(dt);

        if self.capturing.is_some() {
            frame.consumed = true;
            frame.consumed_pointer = true;
            frame.consumed_keyboard = true;
            frame.consumed_text = true;
            frame.capturing = true;
            frame.modal = self.modal.is_some();
            if input.cancel {
                self.cancel_rebind();
                frame.events.push(UiEvent::Cancelled);
            }
            frame.focus = self.focus;
            return frame;
        }

        if !input.scroll.x.is_finite() || !input.scroll.y.is_finite() {
            // Ignore malformed deltas below.
        } else if input.scroll.x != 0.0 || input.scroll.y != 0.0 {
            if let Some(pointer) = input.pointer
                && let Some(id) = self.scrollable_at(pointer)
            {
                let step = self.theme.row_height * scale;
                let node = self.nodes.get_mut(&id).unwrap();
                let content = self.content_sizes.get(&id).copied().unwrap_or(Vec2::ZERO);
                let panel_size = self
                    .layout
                    .iter()
                    .find(|entry| entry.id == id)
                    .map_or(Vec2::ZERO, |entry| {
                        Vec2::new(entry.rect.width, entry.rect.height)
                    });
                let before = node.scroll;
                node.scroll.y = (node.scroll.y - input.scroll.y * step)
                    .clamp(0.0, (content.y - panel_size.y).max(0.0));
                node.scroll.x = (node.scroll.x - input.scroll.x * step)
                    .clamp(0.0, (content.x - panel_size.x).max(0.0));
                if node.scroll != before {
                    self.layout(viewport, scale, safe_area, measure);
                    frame.consumed = true;
                    frame.consumed_pointer = true;
                }
            }
        }

        let hovered = input
            .pointer
            .and_then(|pointer| self.interactive_at(pointer));
        for node in self.nodes.values_mut() {
            node.state.hovered = hovered == Some(node.id);
            if !node.state.hovered {
                node.state.hover_t =
                    (node.state.hover_t - dt * self.theme.transition_rate).max(0.0);
            }
        }
        if let Some(id) = hovered {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.state.hover_t =
                    (node.state.hover_t + dt * self.theme.transition_rate).min(1.0);
            }
            frame.consumed_pointer = true;
        }

        if input.primary_pressed {
            if let Some(pointer) = input.pointer
                && let Some(id) = self.interactive_at(pointer)
            {
                self.pointer_capture = Some(id);
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.state.pressed = true;
                    node.state.press_t = 1.0;
                }
                if self
                    .nodes
                    .get(&id)
                    .is_some_and(|node| node.widget.focusable())
                {
                    self.set_focus_internal(Some(id), &mut frame);
                }
                self.press(id, pointer, input.shift, measure, &mut frame);
                frame.consumed = true;
                frame.consumed_pointer = true;
            }
        }

        if input.primary_down
            && let (Some(id), Some(pointer)) = (self.slider_drag, input.pointer)
        {
            self.drag_slider(id, pointer, &mut frame);
            frame.consumed = true;
            frame.consumed_pointer = true;
        }

        if input.primary_down
            && let (Some(id), Some(pointer)) = (self.text_drag, input.pointer)
            && let Some(position) = self.text_position_at(id, pointer, measure)
            && let Some(node) = self.nodes.get_mut(&id)
            && let Widget::TextInput { cursor, .. } = &mut node.widget
        {
            *cursor = position;
            frame.consumed = true;
            frame.consumed_pointer = true;
        }

        if input.primary_released {
            self.pointer_capture = None;
            self.slider_drag = None;
            self.text_drag = None;
            for node in self.nodes.values_mut() {
                node.state.pressed = false;
            }
        }

        if self.modal.is_some() {
            frame.modal = true;
            frame.consumed_keyboard = true;
        }

        if input.next || input.previous {
            self.move_focus(if input.next { 1 } else { -1 }, &mut frame);
            frame.consumed = true;
            frame.consumed_keyboard = true;
        } else if input.up || input.down {
            if let Some(id) = self.focus
                && matches!(
                    self.nodes.get(&id).map(|node| &node.widget),
                    Some(Widget::List { .. } | Widget::Select { .. } | Widget::Tabs { .. })
                )
            {
                if input.up || input.down {
                    self.adjust_selection(id, if input.down { 1 } else { -1 }, &mut frame);
                }
            } else {
                self.move_focus(if input.down { 1 } else { -1 }, &mut frame);
            }
            frame.consumed = true;
            frame.consumed_keyboard = true;
        }

        if input.left || input.right {
            if let Some(id) = self.focus {
                if matches!(
                    self.nodes.get(&id).map(|node| &node.widget),
                    Some(Widget::Slider { .. } | Widget::Select { .. } | Widget::Tabs { .. })
                ) {
                    self.adjust_selection(id, if input.right { 1 } else { -1 }, &mut frame);
                    frame.consumed = true;
                    frame.consumed_keyboard = true;
                }
            }
        }

        if input.activate {
            if let Some(id) = self.focus {
                self.activate(id, &mut frame);
                frame.consumed = true;
                frame.consumed_keyboard = true;
            }
        }

        if input.cancel {
            if let Some(id) = self.focus
                && matches!(
                    self.nodes.get(&id).map(|node| &node.widget),
                    Some(Widget::TextInput { .. })
                )
            {
                self.focus = None;
                frame.events.push(UiEvent::FocusChanged(None));
            }
            frame.consumed = true;
            frame.consumed_keyboard = true;
            frame.events.push(UiEvent::Cancelled);
        }

        if let Some(id) = self.focus
            && matches!(
                self.nodes.get(&id).map(|node| &node.widget),
                Some(Widget::TextInput { .. })
            )
        {
            self.ensure_text_visible(id, measure);
        }
        for node in self.nodes.values_mut() {
            node.preedit.clear();
            node.preedit_cursor = 0;
        }
        if let Some(id) = self.focus
            && matches!(
                self.nodes.get(&id).map(|node| &node.widget),
                Some(Widget::TextInput { .. })
            )
        {
            if !input.preedit.is_empty() {
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.preedit.clone_from(&input.preedit);
                    node.preedit_cursor = input.preedit_cursor;
                }
                frame.consumed = true;
                frame.consumed_keyboard = true;
                frame.consumed_text = true;
            }
            if input.select_all
                || !input.text.is_empty()
                || input.backspace
                || input.delete
                || input.home
                || input.end
                || input.left
                || input.right
            {
                frame.consumed = true;
                frame.consumed_keyboard = true;
                frame.consumed_text = true;
                self.edit_text(id, input, measure, &mut frame);
                self.ensure_text_visible(id, measure);
            }
        }

        frame.focus = self.focus;
        frame
    }

    fn animate(&mut self, dt: f32) {
        let rate = (dt * self.theme.transition_rate).clamp(0.0, 1.0);
        for node in self.nodes.values_mut() {
            let focus_target = if self.focus == Some(node.id) {
                1.0
            } else {
                0.0
            };
            node.state.focus_t += (focus_target - node.state.focus_t) * rate;
            let press_target = if node.state.pressed { 1.0 } else { 0.0 };
            node.state.press_t += (press_target - node.state.press_t) * rate;
        }
    }

    fn set_focus_internal(&mut self, id: Option<NodeId>, frame: &mut UiFrame) {
        self.focus_initialized = true;
        if self.focus != id {
            self.focus = id;
            frame.focus = id;
            frame.events.push(UiEvent::FocusChanged(id));
        }
    }

    fn move_focus(&mut self, direction: i32, frame: &mut UiFrame) {
        let order: Vec<NodeId> = self
            .focus_order
            .iter()
            .copied()
            .filter(|id| {
                self.modal
                    .is_none_or(|modal| self.is_descendant_or_self(*id, modal))
            })
            .collect();
        if order.is_empty() {
            self.set_focus_internal(None, frame);
            return;
        }
        let index = self
            .focus
            .and_then(|focus| order.iter().position(|id| *id == focus));
        let next = match index {
            Some(index) => ((index as i32 + direction).rem_euclid(order.len() as i32)) as usize,
            None if direction < 0 => order.len() - 1,
            None => 0,
        };
        self.set_focus_internal(Some(order[next]), frame);
    }

    fn first_focusable_in(&self, root: NodeId) -> Option<NodeId> {
        self.focus_order
            .iter()
            .copied()
            .find(|id| self.is_descendant_or_self(*id, root))
    }

    fn is_descendant_or_self(&self, id: NodeId, ancestor: NodeId) -> bool {
        let mut current = Some(id);
        while let Some(node) = current.and_then(|id| self.nodes.get(&id)) {
            if node.id == ancestor {
                return true;
            }
            current = node.parent;
        }
        false
    }

    fn interactive_at(&self, pointer: Vec2) -> Option<NodeId> {
        self.layout
            .iter()
            .rev()
            .find(|entry| {
                entry.clip.contains(pointer)
                    && entry.rect.contains(pointer)
                    && self.nodes.get(&entry.id).is_some_and(|node| {
                        node.visible
                            && node.enabled
                            && node.widget.focusable()
                            && self
                                .modal
                                .is_none_or(|modal| self.is_descendant_or_self(node.id, modal))
                    })
            })
            .map(|entry| entry.id)
    }

    fn scrollable_at(&self, pointer: Vec2) -> Option<NodeId> {
        self.layout
            .iter()
            .rev()
            .find(|entry| {
                entry.rect.contains(pointer)
                    && self.nodes.get(&entry.id).is_some_and(|node| {
                        node.visible
                            && matches!(
                                &node.widget,
                                Widget::Panel {
                                    scrollable: true,
                                    ..
                                }
                            )
                            && self
                                .modal
                                .is_none_or(|modal| self.is_descendant_or_self(node.id, modal))
                    })
            })
            .map(|entry| entry.id)
    }

    fn press(
        &mut self,
        id: NodeId,
        pointer: Vec2,
        shift: bool,
        measure: &dyn TextMeasure,
        frame: &mut UiFrame,
    ) {
        let widget = self.nodes.get(&id).map(|node| node.widget.clone());
        match widget {
            Some(Widget::Button {
                action: Some(action),
                ..
            }) => frame.events.push(UiEvent::Pressed(action)),
            Some(Widget::Checkbox {
                label,
                checked,
                action,
            }) => {
                let checked = !checked;
                if let Some(node) = self.nodes.get_mut(&id)
                    && let Widget::Checkbox { checked: value, .. } = &mut node.widget
                {
                    *value = checked;
                }
                frame.events.push(UiEvent::Changed {
                    id,
                    value: UiValue::Bool(checked),
                });
                if let Some(action) = action {
                    frame.events.push(UiEvent::Pressed(action));
                }
                let _ = label;
            }
            Some(Widget::Slider { .. }) => {
                self.slider_drag = Some(id);
                self.drag_slider(id, pointer, frame);
            }
            Some(Widget::Select { .. } | Widget::Tabs { .. }) => {
                self.adjust_selection(id, 1, frame);
            }
            Some(Widget::List {
                items,
                selected,
                action,
                ..
            }) => {
                if let Some(index) = self.list_index_at(id, pointer) {
                    if let Some(node) = self.nodes.get_mut(&id)
                        && let Widget::List { selected, .. } = &mut node.widget
                    {
                        *selected = index.min(items.len().saturating_sub(1));
                    }
                    frame.events.push(UiEvent::Changed {
                        id,
                        value: UiValue::UInt(index as u32),
                    });
                    if let Some(action) = action {
                        frame.events.push(UiEvent::Pressed(action));
                    }
                } else {
                    let _ = selected;
                }
            }
            Some(Widget::TextInput { .. }) => {
                let position = self
                    .text_position_at(id, pointer, measure)
                    .unwrap_or_else(|| {
                        self.nodes
                            .get(&id)
                            .and_then(|node| match &node.widget {
                                Widget::TextInput { text, .. } => Some(text.chars().count()),
                                _ => None,
                            })
                            .unwrap_or(0)
                    });
                if let Some(node) = self.nodes.get_mut(&id)
                    && let Widget::TextInput { cursor, anchor, .. } = &mut node.widget
                {
                    *cursor = position;
                    if !shift {
                        *anchor = position;
                    }
                }
                self.text_drag = Some(id);
                self.ensure_text_visible(id, measure);
            }
            Some(Widget::Binding { action, .. }) => {
                let _ = self.begin_rebind(id);
                frame.events.push(UiEvent::RebindRequested { id, action });
                frame.capturing = true;
            }
            _ => {}
        }
    }

    fn activate(&mut self, id: NodeId, frame: &mut UiFrame) {
        let widget = self.nodes.get(&id).map(|node| node.widget.clone());
        match widget {
            Some(Widget::Button {
                action: Some(action),
                ..
            }) => frame.events.push(UiEvent::Pressed(action)),
            Some(Widget::Checkbox {
                checked, action, ..
            }) => {
                let checked = !checked;
                if let Some(node) = self.nodes.get_mut(&id)
                    && let Widget::Checkbox { checked: value, .. } = &mut node.widget
                {
                    *value = checked;
                }
                frame.events.push(UiEvent::Changed {
                    id,
                    value: UiValue::Bool(checked),
                });
                if let Some(action) = action {
                    frame.events.push(UiEvent::Pressed(action));
                }
            }
            Some(Widget::Slider {
                value,
                min,
                max,
                step,
                action,
                ..
            }) => {
                let next = (value + step.max(0.01)).min(max);
                let value = if next == value { min } else { next };
                if let Some(node) = self.nodes.get_mut(&id)
                    && let Widget::Slider { value: current, .. } = &mut node.widget
                {
                    *current = value;
                }
                frame.events.push(UiEvent::Changed {
                    id,
                    value: UiValue::Float(value),
                });
                if let Some(action) = action {
                    frame.events.push(UiEvent::Pressed(action));
                }
            }
            Some(Widget::Select {
                selected,
                options,
                action,
                ..
            }) => {
                let selected = (selected + 1) % options.len().max(1);
                if let Some(node) = self.nodes.get_mut(&id)
                    && let Widget::Select {
                        selected: value, ..
                    } = &mut node.widget
                {
                    *value = selected;
                }
                frame.events.push(UiEvent::Changed {
                    id,
                    value: UiValue::UInt(selected as u32),
                });
                if let Some(action) = action {
                    frame.events.push(UiEvent::Pressed(action));
                }
            }
            Some(Widget::Tabs {
                selected,
                tabs,
                action,
            }) => {
                let selected = (selected + 1) % tabs.len().max(1);
                if let Some(node) = self.nodes.get_mut(&id)
                    && let Widget::Tabs {
                        selected: value, ..
                    } = &mut node.widget
                {
                    *value = selected;
                }
                frame.events.push(UiEvent::Changed {
                    id,
                    value: UiValue::UInt(selected as u32),
                });
                if let Some(action) = action {
                    frame.events.push(UiEvent::Pressed(action));
                }
            }
            Some(Widget::List {
                action: Some(action),
                ..
            }) => frame.events.push(UiEvent::Pressed(action)),
            Some(Widget::TextInput { text, action, .. }) => {
                frame.events.push(UiEvent::Submitted {
                    id,
                    text: text.clone(),
                });
                if let Some(action) = action {
                    frame.events.push(UiEvent::Pressed(action));
                }
            }
            Some(Widget::Binding { action, .. }) => {
                let _ = self.begin_rebind(id);
                frame.events.push(UiEvent::RebindRequested { id, action });
                frame.capturing = true;
            }
            _ => {}
        }
    }

    fn adjust_selection(&mut self, id: NodeId, direction: i32, frame: &mut UiFrame) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        match &mut node.widget {
            Widget::Slider {
                value,
                min,
                max,
                step,
                action,
                ..
            } => {
                let step = step.max(0.01) * direction as f32;
                let next = (*value + step).clamp(*min, *max);
                *value = next;
                frame.events.push(UiEvent::Changed {
                    id,
                    value: UiValue::Float(next),
                });
                if let Some(action) = *action {
                    frame.events.push(UiEvent::Pressed(action));
                }
            }
            Widget::Select {
                selected,
                options,
                action,
                ..
            } => {
                if !options.is_empty() {
                    *selected =
                        (*selected as i32 + direction).rem_euclid(options.len() as i32) as usize;
                    let value = *selected;
                    frame.events.push(UiEvent::Changed {
                        id,
                        value: UiValue::UInt(value as u32),
                    });
                    if let Some(action) = *action {
                        frame.events.push(UiEvent::Pressed(action));
                    }
                }
            }
            Widget::Tabs {
                selected,
                tabs,
                action,
            } => {
                if !tabs.is_empty() {
                    *selected =
                        (*selected as i32 + direction).rem_euclid(tabs.len() as i32) as usize;
                    let value = *selected;
                    frame.events.push(UiEvent::Changed {
                        id,
                        value: UiValue::UInt(value as u32),
                    });
                    if let Some(action) = *action {
                        frame.events.push(UiEvent::Pressed(action));
                    }
                }
            }
            Widget::List {
                selected,
                items,
                action,
                ..
            } if !items.is_empty() => {
                *selected = (*selected as i32 + direction).rem_euclid(items.len() as i32) as usize;
                frame.events.push(UiEvent::Changed {
                    id,
                    value: UiValue::UInt(*selected as u32),
                });
                if let Some(action) = *action {
                    frame.events.push(UiEvent::Pressed(action));
                }
            }
            _ => {}
        }
    }

    fn drag_slider(&mut self, id: NodeId, pointer: Vec2, frame: &mut UiFrame) {
        let Some(entry) = self.layout.iter().find(|entry| entry.id == id) else {
            return;
        };
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if let Widget::Slider {
            value,
            min,
            max,
            step,
            action,
            ..
        } = &mut node.widget
        {
            let width = entry.rect.width.max(1.0);
            let t = ((pointer.x - entry.rect.x) / width).clamp(0.0, 1.0);
            let raw = *min + (*max - *min) * t;
            let next = if *step > 0.0 {
                (*min + ((raw - *min) / *step).round() * *step).clamp(*min, *max)
            } else {
                raw
            };
            if (*value - next).abs() > f32::EPSILON {
                *value = next;
                frame.events.push(UiEvent::Changed {
                    id,
                    value: UiValue::Float(next),
                });
                if let Some(action) = *action {
                    frame.events.push(UiEvent::Pressed(action));
                }
            }
        }
    }

    fn list_index_at(&self, id: NodeId, pointer: Vec2) -> Option<usize> {
        let entry = self.layout.iter().find(|entry| entry.id == id)?;
        let node = self.nodes.get(&id)?;
        let Widget::List { items, scroll, .. } = &node.widget else {
            return None;
        };
        let row = self.theme.row_height * self.scale;
        let index = ((pointer.y - entry.rect.y) / row).floor() as usize + *scroll;
        (index < items.len()).then_some(index)
    }

    fn text_position_at(
        &self,
        id: NodeId,
        pointer: Vec2,
        measure: &dyn TextMeasure,
    ) -> Option<usize> {
        let entry = self.layout.iter().find(|entry| entry.id == id)?;
        let node = self.nodes.get(&id)?;
        let Widget::TextInput { text, .. } = &node.widget else {
            return None;
        };
        let size = self.theme.font_size * self.scale;
        let padding = self.theme.padding * self.scale;
        let x = pointer.x - (entry.rect.x + padding) + node.text_scroll;
        if let Some(index) = measure.caret_index(text, crate::TextStyle::Body, size, x) {
            return Some(index);
        }
        let mut best = 0;
        let mut best_distance = f32::MAX;
        let mut width = 0.0;
        for (index, ch) in text.chars().enumerate() {
            let distance = (width - x).abs();
            if distance < best_distance {
                best_distance = distance;
                best = index;
            }
            width += measure
                .measure_style(&ch.to_string(), crate::TextStyle::Body, size)
                .x;
        }
        if (width - x).abs() < best_distance {
            best = text.chars().count();
        }
        Some(best)
    }

    fn ensure_text_visible(&mut self, id: NodeId, measure: &dyn TextMeasure) {
        let Some(entry) = self.layout.iter().find(|entry| entry.id == id) else {
            return;
        };
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        let Widget::TextInput { text, cursor, .. } = &node.widget else {
            return;
        };
        let size = self.theme.font_size * self.scale;
        let Some(caret) = measure.caret_x(text, crate::TextStyle::Body, size, *cursor) else {
            return;
        };
        let padding = self.theme.padding * self.scale;
        let inner = (entry.rect.width - padding * 2.0).max(1.0);
        let total = measure.measure_style(text, crate::TextStyle::Body, size).x;
        let max_scroll = (total - inner).max(0.0);
        let mut scroll = node.text_scroll;
        if caret - scroll < 0.0 {
            scroll = caret;
        } else if caret - scroll > inner {
            scroll = caret - inner;
        }
        if let Some(node) = self.nodes.get_mut(&id) {
            node.text_scroll = scroll.clamp(0.0, max_scroll);
        }
    }

    fn edit_text(
        &mut self,
        id: NodeId,
        input: &UiInput,
        measure: &dyn TextMeasure,
        frame: &mut UiFrame,
    ) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        let Widget::TextInput {
            text,
            cursor: cursor_field,
            anchor: anchor_field,
            max_len,
            ..
        } = &mut node.widget
        else {
            return;
        };
        let char_len = text.chars().count();
        let mut cursor = (*cursor_field).min(char_len);
        let mut anchor = (*anchor_field).min(char_len);
        if input.select_all {
            cursor = char_len;
            anchor = 0;
        }
        if input.home {
            cursor = 0;
            if !input.shift {
                anchor = 0;
            }
        }
        if input.end {
            cursor = char_len;
            if !input.shift {
                anchor = char_len;
            }
        }
        if input.left {
            cursor = measure.previous_boundary(text, cursor);
            if !input.shift {
                anchor = cursor;
            }
        }
        if input.right {
            cursor = measure.next_boundary(text, cursor);
            if !input.shift {
                anchor = cursor;
            }
        }
        if input.backspace || input.delete {
            let (start, end) = if cursor != anchor {
                (cursor.min(anchor), cursor.max(anchor))
            } else if input.backspace && cursor > 0 {
                (measure.previous_boundary(text, cursor), cursor)
            } else if input.delete && cursor < char_len {
                (cursor, measure.next_boundary(text, cursor))
            } else {
                (cursor, cursor)
            };
            if start != end {
                let start_byte = byte_index(text, start);
                let end_byte = byte_index(text, end);
                text.replace_range(start_byte..end_byte, "");
                cursor = start;
                anchor = start;
            }
        }
        if !input.text.is_empty() {
            let (start, end) = (cursor.min(anchor), cursor.max(anchor));
            let start_byte = byte_index(text, start);
            let end_byte = byte_index(text, end);
            let remaining =
                max_len.saturating_sub(text.chars().count() + input.text.chars().count());
            if remaining > 0 || start != end {
                let filtered: String = input
                    .text
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(max_len.saturating_sub(text.chars().count().saturating_sub(end - start)))
                    .collect();
                text.replace_range(start_byte..end_byte, &filtered);
                cursor = start + filtered.chars().count();
                anchor = cursor;
            }
        }
        *cursor_field = cursor.min(text.chars().count());
        *anchor_field = anchor.min(text.chars().count());
        let value = text.clone();
        frame.events.push(UiEvent::TextChanged { id, text: value });
    }
}

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

struct LayoutPass<'a> {
    ui: &'a Ui,
    measure: &'a dyn TextMeasure,
    scale: f32,
    entries: Vec<LayoutEntry>,
    content_sizes: BTreeMap<NodeId, Vec2>,
}

impl LayoutPass<'_> {
    fn resolve_root_size(&self, id: NodeId, area: Rect) -> Vec2 {
        let node = &self.ui.nodes[&id];
        let intrinsic = self.intrinsic(id, area.width, area.height);
        let width = resolve(node.item.width, area.width, intrinsic.x, self.scale);
        let height = resolve(node.item.height, area.height, intrinsic.y, self.scale);
        Vec2::new(width, height)
    }

    fn layout_node(&mut self, id: NodeId, rect: Rect, clip: Rect, parent: Rect) {
        let Some(node) = self.ui.nodes.get(&id) else {
            return;
        };
        if !node.visible {
            return;
        }
        let own_clip = clip.intersection(rect);
        self.entries.push(LayoutEntry {
            id,
            rect,
            clip: own_clip,
            layer: node.layer,
        });
        let content_clip = if matches!(node.widget, Widget::Panel { clip: true, .. })
            || matches!(
                node.widget,
                Widget::Panel {
                    scrollable: true,
                    ..
                }
            ) {
            own_clip
        } else {
            clip
        };
        let Widget::Panel {
            layout,
            padding,
            scrollable,
            ..
        } = &node.widget
        else {
            return;
        };
        let inner = rect.inset(padding.scaled(self.scale));
        let offset = if *scrollable {
            Vec2::new(-node.scroll.x, -node.scroll.y)
        } else {
            Vec2::ZERO
        };
        let mut content = Vec2::new(inner.width, inner.height);
        match layout {
            PanelLayout::Stack { direction, gap } => {
                self.layout_stack(
                    &node.children,
                    inner,
                    content_clip,
                    *direction,
                    *gap * self.scale,
                    offset,
                    &mut content,
                );
            }
            PanelLayout::Grid { columns, gap } => {
                self.layout_grid(
                    &node.children,
                    inner,
                    content_clip,
                    *columns,
                    *gap * self.scale,
                    offset,
                    &mut content,
                );
            }
            PanelLayout::Absolute => {
                for child in &node.children {
                    self.layout_absolute(*child, inner, content_clip, offset, &mut content);
                }
            }
        }
        self.content_sizes.insert(id, content);
        let _ = parent;
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_stack(
        &mut self,
        children: &[NodeId],
        area: Rect,
        clip: Rect,
        direction: StackDirection,
        gap: f32,
        offset: Vec2,
        content: &mut Vec2,
    ) {
        let visible: Vec<NodeId> = children
            .iter()
            .copied()
            .filter(|id| self.ui.nodes.get(id).is_some_and(|node| node.visible))
            .collect();
        if visible.is_empty() {
            return;
        }
        let vertical = direction == StackDirection::Vertical;
        let available_main = if vertical { area.height } else { area.width };
        let available_cross = if vertical { area.width } else { area.height };
        let mut sizes = Vec::with_capacity(visible.len());
        let mut main_total = 0.0;
        let mut fill_count = 0;
        for id in &visible {
            let node = &self.ui.nodes[id];
            let margin = node.item.margin.scaled(self.scale);
            let cross_available = available_cross
                - (if vertical {
                    margin.left + margin.right
                } else {
                    margin.top + margin.bottom
                });
            let intrinsic = self.intrinsic(*id, cross_available, available_main);
            let length = if vertical {
                node.item.height
            } else {
                node.item.width
            };
            let resolved = match length {
                Length::Fill => {
                    fill_count += 1;
                    0.0
                }
                _ => resolve(
                    length,
                    available_main,
                    if vertical { intrinsic.y } else { intrinsic.x },
                    self.scale,
                ),
            };
            main_total += resolved
                + if vertical {
                    margin.top + margin.bottom
                } else {
                    margin.left + margin.right
                };
            sizes.push((*id, intrinsic, resolved, margin));
        }
        let remaining =
            (available_main - main_total - gap * (visible.len().saturating_sub(1)) as f32).max(0.0);
        let fill_size = if fill_count > 0 {
            remaining / fill_count as f32
        } else {
            0.0
        };
        let mut cursor =
            if vertical { area.y } else { area.x } + if vertical { offset.y } else { offset.x };
        for (id, intrinsic, resolved, margin) in sizes {
            let main = if resolved == 0.0 && fill_count > 0 {
                fill_size.max(0.0)
            } else {
                resolved
            };
            let node = &self.ui.nodes[&id];
            let cross = if vertical {
                resolve(
                    node.item.width,
                    available_cross - margin.left - margin.right,
                    intrinsic.x,
                    self.scale,
                )
            } else {
                resolve(
                    node.item.height,
                    available_cross - margin.top - margin.bottom,
                    intrinsic.y,
                    self.scale,
                )
            };
            let cross_area = if vertical { area.width } else { area.height };
            let cross_offset = if vertical { margin.left } else { margin.top };
            let cross_size = if cross == 0.0 {
                (cross_area
                    - cross_offset
                    - if vertical {
                        margin.right
                    } else {
                        margin.bottom
                    })
                .max(0.0)
            } else {
                cross
            };
            let start = if vertical {
                area.x + cross_offset
            } else {
                area.y + cross_offset
            };
            let align = node.item.align;
            let cross_position = match align {
                Align::Start | Align::Stretch => start,
                Align::Center => start + (cross_area - cross_offset - cross_size) * 0.5,
                Align::End => {
                    area.x + area.width
                        - cross_size
                        - if vertical {
                            margin.right
                        } else {
                            margin.bottom
                        }
                }
            };
            let rect = if vertical {
                Rect::new(
                    cross_position + node.item.offset.x * self.scale,
                    cursor + margin.top,
                    cross_size + offset.x * 0.0,
                    main,
                )
            } else {
                Rect::new(
                    cursor + margin.left,
                    cross_position + node.item.offset.y * self.scale,
                    main,
                    cross_size,
                )
            };
            let rect = rect.translated(if vertical {
                Vec2::new(offset.x, 0.0)
            } else {
                Vec2::new(0.0, offset.y)
            });
            self.layout_node(id, rect, clip, area);
            cursor += main
                + gap
                + if vertical {
                    margin.top + margin.bottom
                } else {
                    margin.left + margin.right
                };
            if vertical {
                content.y = content.y.max(cursor - area.y);
            } else {
                content.x = content.x.max(cursor - area.x);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_grid(
        &mut self,
        children: &[NodeId],
        area: Rect,
        clip: Rect,
        columns: u32,
        gap: f32,
        offset: Vec2,
        content: &mut Vec2,
    ) {
        let columns = columns.max(1) as usize;
        let gap_total = gap * (columns.saturating_sub(1)) as f32;
        let cell = ((area.width - gap_total) / columns as f32).max(0.0);
        let mut row = 0usize;
        let mut y = area.y + offset.y;
        while row * columns < children.len() {
            let mut row_height = 0.0f32;
            for column in 0..columns {
                if let Some(id) = children.get(row * columns + column)
                    && let Some(node) = self.ui.nodes.get(id)
                    && node.visible
                {
                    let intrinsic = self.intrinsic(*id, cell, area.height);
                    let height = resolve(node.item.height, area.height, intrinsic.y, self.scale);
                    row_height = row_height.max(
                        height
                            + node.item.margin.scaled(self.scale).top
                            + node.item.margin.scaled(self.scale).bottom,
                    );
                }
            }
            row_height = row_height.max(self.ui.theme.row_height * self.scale);
            for column in 0..columns {
                let Some(id) = children.get(row * columns + column).copied() else {
                    break;
                };
                let node = &self.ui.nodes[&id];
                if !node.visible {
                    continue;
                }
                let margin = node.item.margin.scaled(self.scale);
                let intrinsic = self.intrinsic(id, cell, row_height);
                let width = resolve(node.item.width, cell, intrinsic.x, self.scale);
                let height = resolve(node.item.height, row_height, intrinsic.y, self.scale);
                let rect = Rect::new(
                    area.x + offset.x + column as f32 * (cell + gap) + margin.left,
                    y + margin.top,
                    width.max(0.0),
                    height.max(0.0),
                );
                self.layout_node(id, rect, clip, area);
            }
            y += row_height + gap;
            content.y = content.y.max(y - area.y);
            row += 1;
        }
        content.x = content.x.max(area.width);
    }

    fn layout_absolute(
        &mut self,
        id: NodeId,
        area: Rect,
        clip: Rect,
        offset: Vec2,
        content: &mut Vec2,
    ) {
        let Some(node) = self.ui.nodes.get(&id) else {
            return;
        };
        if !node.visible {
            return;
        }
        let margin = node.item.margin.scaled(self.scale);
        let intrinsic = self.intrinsic(id, area.width, area.height);
        let width = resolve(node.item.width, area.width, intrinsic.x, self.scale);
        let height = resolve(node.item.height, area.height, intrinsic.y, self.scale);
        let x = match node.item.align {
            Align::Center => area.x + (area.width - width).max(0.0) * 0.5,
            Align::End => area.x + area.width - width - margin.right,
            Align::Start | Align::Stretch => area.x + margin.left,
        };
        let y = match node.item.align {
            Align::Center => area.y + (area.height - height).max(0.0) * 0.5,
            Align::End => area.y + area.height - height - margin.bottom,
            Align::Start | Align::Stretch => area.y + margin.top,
        };
        let rect = Rect::new(
            x + offset.x + node.item.offset.x * self.scale,
            y + offset.y + node.item.offset.y * self.scale,
            width,
            height,
        );
        content.x = content.x.max(rect.right() - area.x);
        content.y = content.y.max(rect.bottom() - area.y);
        self.layout_node(id, rect, clip, area);
    }

    fn intrinsic(&self, id: NodeId, available_width: f32, available_height: f32) -> Vec2 {
        let Some(node) = self.ui.nodes.get(&id) else {
            return Vec2::ZERO;
        };
        let font = self.ui.theme.font_size * self.scale;
        match &node.widget {
            Widget::Panel {
                layout, padding, ..
            } => {
                let inner = Vec2::new(
                    (available_width
                        - padding.scaled(self.scale).left
                        - padding.scaled(self.scale).right)
                        .max(0.0),
                    (available_height
                        - padding.scaled(self.scale).top
                        - padding.scaled(self.scale).bottom)
                        .max(0.0),
                );
                let mut size = Vec2::ZERO;
                match layout {
                    PanelLayout::Stack { direction, gap } => {
                        let vertical = *direction == StackDirection::Vertical;
                        for child in &node.children {
                            if let Some(child_node) = self.ui.nodes.get(child)
                                && child_node.visible
                            {
                                let child_size = self.intrinsic(*child, inner.x, inner.y);
                                if vertical {
                                    size.x = size.x.max(child_size.x);
                                    size.y += child_size.y;
                                } else {
                                    size.y = size.y.max(child_size.y);
                                    size.x += child_size.x;
                                }
                            }
                        }
                        if vertical {
                            size.y +=
                                gap * self.scale * node.children.len().saturating_sub(1) as f32;
                        } else {
                            size.x +=
                                gap * self.scale * node.children.len().saturating_sub(1) as f32;
                        }
                    }
                    PanelLayout::Grid { columns, gap } => {
                        let columns = (*columns).max(1) as usize;
                        let mut row_heights: Vec<f32> = Vec::new();
                        for (index, child) in node.children.iter().enumerate() {
                            if let Some(child_node) = self.ui.nodes.get(child)
                                && child_node.visible
                            {
                                let child_size =
                                    self.intrinsic(*child, inner.x / columns as f32, inner.y);
                                let row = index / columns;
                                if row_heights.len() <= row {
                                    row_heights.push(0.0);
                                }
                                row_heights[row] = row_heights[row].max(child_size.y);
                                size.x = size.x.max(
                                    child_size.x * columns as f32
                                        + gap * self.scale * (columns as f32 - 1.0),
                                );
                            }
                        }
                        size.y = row_heights.iter().sum::<f32>()
                            + gap * self.scale * row_heights.len().saturating_sub(1) as f32;
                    }
                    PanelLayout::Absolute => {
                        for child in &node.children {
                            if let Some(child_node) = self.ui.nodes.get(child)
                                && child_node.visible
                            {
                                let child_size = self.intrinsic(*child, inner.x, inner.y);
                                size.x = size.x.max(child_size.x);
                                size.y = size.y.max(child_size.y);
                            }
                        }
                    }
                }
                size.x += padding.scaled(self.scale).left + padding.scaled(self.scale).right;
                size.y += padding.scaled(self.scale).top + padding.scaled(self.scale).bottom;
                size
            }
            Widget::Label { text, style } => self.measure.measure_style(
                self.ui.resolve_text(text),
                *style,
                self.ui.theme.size(*style) * self.scale,
            ),
            Widget::Button { text, .. } => {
                let text_size = self.measure.measure_style(
                    self.ui.resolve_text(text),
                    crate::TextStyle::Body,
                    font,
                );
                Vec2::new(
                    text_size.x + font * 2.0,
                    (text_size.y + font).max(self.ui.theme.row_height * self.scale),
                )
            }
            Widget::Slider { label, .. } => {
                let text_size = self.measure.measure_style(
                    self.ui.resolve_text(label),
                    crate::TextStyle::Body,
                    font,
                );
                Vec2::new(
                    text_size.x.max(220.0 * self.scale),
                    self.ui.theme.row_height * self.scale,
                )
            }
            Widget::Checkbox { label, .. } => {
                let text_size = self.measure.measure_style(
                    self.ui.resolve_text(label),
                    crate::TextStyle::Body,
                    font,
                );
                Vec2::new(
                    text_size.x + font * 1.8,
                    self.ui.theme.row_height * self.scale,
                )
            }
            Widget::Select {
                label,
                options,
                selected,
                ..
            } => {
                let text_size = self.measure.measure_style(
                    self.ui.resolve_text(label),
                    crate::TextStyle::Body,
                    font,
                );
                let option = options
                    .get(*selected)
                    .map(|text| {
                        self.measure
                            .measure_style(self.ui.resolve_text(text), crate::TextStyle::Body, font)
                            .x
                    })
                    .unwrap_or(0.0);
                Vec2::new(
                    text_size.x.max(180.0 * self.scale) + option,
                    self.ui.theme.row_height * self.scale,
                )
            }
            Widget::Tabs { tabs, .. } => {
                let width = tabs
                    .iter()
                    .map(|text| {
                        self.measure
                            .measure_style(self.ui.resolve_text(text), crate::TextStyle::Body, font)
                            .x
                            + font
                    })
                    .sum::<f32>();
                Vec2::new(width, self.ui.theme.row_height * self.scale)
            }
            Widget::TextInput {
                text, placeholder, ..
            } => {
                let value = if text.is_empty() {
                    self.ui.resolve_text(placeholder)
                } else {
                    text.as_str()
                };
                Vec2::new(
                    self.measure
                        .measure_style(value, crate::TextStyle::Body, font)
                        .x
                        .max(200.0 * self.scale),
                    self.ui.theme.row_height * self.scale,
                )
            }
            Widget::Progress { label, .. } => Vec2::new(
                self.measure
                    .measure_style(self.ui.resolve_text(label), crate::TextStyle::Small, font)
                    .x
                    .max(200.0 * self.scale),
                self.ui.theme.row_height * self.scale,
            ),
            Widget::Image { size, .. } => *size * self.scale,
            Widget::List { items, .. } => Vec2::new(
                items
                    .iter()
                    .map(|text| {
                        self.measure
                            .measure_style(self.ui.resolve_text(text), crate::TextStyle::Body, font)
                            .x
                    })
                    .fold(180.0 * self.scale, f32::max),
                self.ui.theme.row_height * self.scale * items.len().max(1) as f32,
            ),
            Widget::Binding { label, value, .. } => {
                let width = self
                    .measure
                    .measure_style(self.ui.resolve_text(label), crate::TextStyle::Body, font)
                    .x
                    + self
                        .measure
                        .measure_style(value, crate::TextStyle::Body, font)
                        .x;
                Vec2::new(width + font * 2.0, self.ui.theme.row_height * self.scale)
            }
            Widget::Spacer => Vec2::ZERO,
        }
    }
}

fn resolve(length: Length, available: f32, intrinsic: f32, scale: f32) -> f32 {
    match length {
        Length::Auto => intrinsic,
        Length::Fixed(value) => value * scale,
        Length::Percent(value) => available * value.clamp(0.0, 1.0),
        Length::Fill => available,
    }
}
