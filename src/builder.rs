//! Far's DialogBuilder (FarDlgBuilder.cpp, DlgBuilder.hpp), which lays out
//! most of the Options dialogs: items one under another from (5, 2), a
//! text before or after an item on its line, Indent, two columns, a
//! separator, OK and Cancel; the box as wide as the widest item (a text's
//! width counts its `&`, as Far's strData.size() does) plus 10. Each item
//! is bound to a setting (`crate::settings`): OK writes them all.

use crate::dialog::{self, Action, Dialog, Item};
use crate::lang::tr;
use crate::settings;

/// Far's SECOND_COLUMN marker.
const SECOND: i16 = -2;

/// How an item's value goes to a setting.
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    Bool(&'static str),
    /// A check box for one bit of an integer setting: its id, the setting, the bit.
    Mask(&'static str, &'static str, i64),
    Int(&'static str),
    Str(&'static str),
    /// A drop-down list: the setting and each item's value.
    Combo(&'static str, Vec<i64>),
    /// Radio buttons: the group's setting (the index is the value).
    Radio(&'static str),
}

struct Placed {
    item: Item,
    x1: i16,
    x2: i16,
    y: u16,
}

pub struct Builder {
    title: String,
    items: Vec<Placed>,
    bindings: Vec<Binding>,
    links: Vec<(usize, usize)>,
    next_y: u16,
    column_start: Option<usize>,
    column_break: Option<usize>,
    column_start_y: u16,
    column_end_y: u16,
    column_min_width: i16,
}

fn chars(text: &str) -> i16 {
    text.chars().count() as i16
}

impl Builder {
    pub fn new(title: &'static str) -> Self {
        Self {
            title: tr(title).to_string(),
            items: vec![],
            bindings: vec![],
            links: vec![],
            next_y: 2,
            column_start: None,
            column_break: None,
            column_start_y: 0,
            column_end_y: 0,
            column_min_width: 0,
        }
    }

    /// Far's ItemWidth.
    fn width(item: &Placed) -> i16 {
        match &item.item {
            Item::Label(text) | Item::Text { text, .. } => chars(text),
            Item::Check { label, .. } | Item::Option { label, .. } | Item::Check3 { label, .. } => chars(label) + 4,
            Item::Edit { .. } | Item::Combo { .. } => item.x2 - item.x1 + 1,
            _ => 0,
        }
    }

    fn push(&mut self, item: Item, width: i16) -> usize {
        let y = self.next_y;
        self.next_y += 1;
        let mut placed = Placed { item, x1: 5, x2: 0, y };
        placed.x2 = placed.x1 + width;
        self.items.push(placed);
        self.items.len() - 1
    }

    /// AddText.
    pub fn text(&mut self, label: &'static str) -> usize {
        self.push(Item::Label(tr(label).into()), 0)
    }

    /// AddCheckbox bound to a yes/no setting.
    pub fn check(&mut self, key: &'static str, label: &'static str) -> usize {
        self.bindings.push(Binding::Bool(key));
        let label = tr(label);
        let w = chars(label) + 4;
        self.push(Item::Check { id: key, label, checked: settings::bool(key), enabled: true }, w)
    }

    /// AddCheckbox for one bit of an integer setting.
    pub fn check_bit(&mut self, id: &'static str, key: &'static str, bit: i64, label: &'static str) -> usize {
        self.bindings.push(Binding::Mask(id, key, bit));
        let label = tr(label);
        let w = chars(label) + 4;
        self.push(Item::Check { id, label, checked: settings::int(key) & bit != 0, enabled: true }, w)
    }

    fn edit_item(id: &'static str, value: &str) -> Item {
        let text: Vec<char> = value.chars().collect();
        let cursor = text.len();
        Item::Edit { id, label: None, x: 0, text, cursor, unchanged: !value.is_empty(), enabled: true, history: false, secret: false }
    }

    /// AddIntEditField: `width` columns.
    pub fn int_edit(&mut self, key: &'static str, width: i16) -> usize {
        self.bindings.push(Binding::Int(key));
        let index = self.push(Self::edit_item(key, &settings::int(key).to_string()), 0);
        self.items[index].x2 = self.items[index].x1 + width - 1;
        index
    }

    /// AddEditField: X2 = X1 + Width.
    pub fn edit(&mut self, key: &'static str, width: i16) -> usize {
        self.bindings.push(Binding::Str(key));
        let index = self.push(Self::edit_item(key, &settings::string(key)), 0);
        self.items[index].x2 = self.items[index].x1 + width;
        index
    }

    /// AddComboBox: the items' texts and values.
    pub fn combo(&mut self, key: &'static str, width: i16, options: &[(&'static str, i64)]) -> usize {
        let values: Vec<i64> = options.iter().map(|(_, v)| *v).collect();
        let current = settings::int(key);
        let selected = values.iter().position(|v| *v == current).unwrap_or(0);
        self.bindings.push(Binding::Combo(key, values));
        let labels = options.iter().map(|(t, _)| tr(t)).collect();
        let index = self.push(Item::Combo { id: key, label: "", x: 0, options: labels, selected, unchanged: true }, 0);
        self.items[index].x2 = self.items[index].x1 + width;
        index
    }

    /// AddRadioButtons: the setting is the index of the one that is on.
    pub fn radio(&mut self, key: &'static str, labels: &[&'static str]) -> usize {
        self.bindings.push(Binding::Radio(key));
        let current = settings::int(key);
        let mut first = 0;
        for (i, label) in labels.iter().enumerate() {
            let label = tr(label);
            let w = chars(label) + 4;
            let index = self.push(Item::Option { id: key, index: i, label, on: current == i as i64 }, w);
            if i == 0 {
                first = index;
            }
        }
        first
    }

    /// AddTextBefore: the label at the line's start, the item after it.
    pub fn text_before(&mut self, target: usize, label: &'static str) -> usize {
        let label = tr(label);
        let (y, indent) = (self.items[target].y, self.items[target].x1 - 5);
        let x1 = 5 + indent.max(0);
        let x2 = x1 + chars(label) - 1;
        let target_width = self.items[target].x2 - self.items[target].x1;
        self.items[target].x1 = x2 + 2;
        self.items[target].x2 = self.items[target].x1 + target_width;
        self.items.push(Placed { item: Item::Label(label.into()), x1, x2, y });
        self.items.len() - 1
    }

    /// AddTextAfter (one column after the item).
    pub fn text_after(&mut self, target: usize, label: &'static str) -> usize {
        let label = tr(label);
        let y = self.items[target].y;
        let x1 = self.items[target].x1 + Self::width(&self.items[target]) + 1;
        self.items.push(Placed { item: Item::Label(label.into()), x1, x2: x1 + chars(label), y });
        self.items.len() - 1
    }

    /// Indent.
    pub fn indent(&mut self, index: usize, by: i16) {
        self.items[index].x1 += by;
        self.items[index].x2 += by;
    }

    /// LinkFlags(parent, target, DIF_DISABLE).
    pub fn link(&mut self, parent: usize, target: usize) {
        self.links.push((parent, target));
    }

    pub fn separator(&mut self, text: Option<&'static str>) {
        let y = self.next_y;
        self.next_y += 1;
        self.items.push(Placed { item: Item::Separator(text.map(tr)), x1: -1, x2: 0, y });
    }

    pub fn start_columns(&mut self) {
        self.column_start = Some(self.items.len());
        self.column_start_y = self.next_y;
    }

    pub fn column_break(&mut self) {
        self.column_break = Some(self.items.len());
        self.column_end_y = self.next_y;
        self.next_y = self.column_start_y;
    }

    pub fn end_columns(&mut self) {
        let (Some(start), Some(brk)) = (self.column_start.take(), self.column_break.take()) else { return };
        for i in start..self.items.len() {
            let width = Self::width(&self.items[i]);
            self.column_min_width = self.column_min_width.max(width);
            if i >= brk {
                self.items[i].x1 = SECOND;
                self.items[i].x2 = SECOND + width;
            }
        }
        self.next_y = self.next_y.max(self.column_end_y);
    }

    /// AddOKCancel: a separator, OK and Cancel.
    pub fn ok_cancel(&mut self) {
        self.separator(None);
        let y = self.next_y;
        self.next_y += 1;
        let buttons = vec![dialog::button(tr("OK"), "ok", true), dialog::button(tr("Cancel"), "cancel", false)];
        self.items.push(Placed { item: Item::Buttons(buttons), x1: -1, x2: 0, y });
    }

    /// The dialog: UpdateBorderSize and UpdateSecondColumnPosition, then
    /// every item at its place. `action` gets the bindings.
    pub fn build(self, kind: Settings) -> Dialog {
        let mut max_width = self.items.iter().filter(|p| p.x1 != SECOND && p.x1 >= 0).map(|p| Self::width(p) + p.x1 - 5).max().unwrap_or(0);
        max_width = max_width.max(2 * self.column_min_width + 1);
        let box_x2 = 3 + max_width + 3;
        let height = self.items.iter().map(|p| p.y).max().unwrap_or(2) + 1 + 2;
        let second_x1 = 6 + (box_x2 - 3 - 1) / 2;
        let width = (box_x2 + 4) as u16;
        let mut items = Vec::new();
        let mut places = Vec::new();
        for mut p in self.items {
            if p.x1 == SECOND {
                let w = p.x2 - p.x1;
                p.x1 = second_x1;
                p.x2 = second_x1 + w;
            }
            let place = match p.item {
                Item::Edit { .. } | Item::Combo { .. } => dialog::span(p.x1, p.y, p.x2 as u16),
                Item::Separator(_) | Item::Buttons(_) => dialog::at(-1, p.y),
                _ => dialog::at(p.x1, p.y),
            };
            items.push(p.item);
            places.push(place);
        }
        let focus = items.iter().position(|i| !matches!(i, Item::Label(_) | Item::Text { .. } | Item::Separator(_))).unwrap_or(0);
        let mut dialog = Dialog::placed(self.title, (width, height), items, places, focus, None, Action::Settings(kind, self.bindings));
        for (parent, target) in self.links {
            dialog.link_flags(parent, target);
        }
        dialog.apply_links();
        dialog
    }
}

/// Which Options dialog: what changes when it is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settings {
    System,
    Panel,
    Interface,
    Dialog,
    Menu,
    Cmdline,
    AutoComplete,
    InfoPanel,
    Confirmations,
    Descriptions,
    Viewer,
}

/// OK: every bound item's value into its setting.
pub fn save(bindings: &[Binding], values: &dialog::Values) {
    for binding in bindings {
        match binding {
            Binding::Bool(key) => settings::set_bool(key, values.checked(key)),
            Binding::Mask(id, key, bit) => {
                let current = settings::int(key);
                let new = if values.checked(id) { current | bit } else { current & !bit };
                settings::set_int(key, new);
            }
            Binding::Int(key) => {
                if let Ok(n) = values.text(key).trim().parse::<i64>() {
                    settings::set_int(key, n);
                }
            }
            Binding::Str(key) => settings::set_string(key, &values.text(key)),
            Binding::Combo(key, list) => {
                if let Some(v) = list.get(values.choice(key)) {
                    settings::set_int(key, *v);
                }
            }
            Binding::Radio(key) => settings::set_int(key, values.choice(key) as i64),
        }
    }
}
