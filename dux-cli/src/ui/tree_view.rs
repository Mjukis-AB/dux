use std::collections::HashSet;

use dux_core::{DiskTree, NodeId, NodeKind, format_size, size_percentage};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};

use super::bar_chart::render_bar;
use super::theme::Theme;

/// Tree prefix characters
struct TreePrefix;

impl TreePrefix {
    const PIPE: &'static str = "│  ";
    const TEE: &'static str = "├─ ";
    const CORNER: &'static str = "└─ ";
    const BLANK: &'static str = "   ";
}

/// Main tree view widget
pub struct TreeView<'a> {
    tree: &'a DiskTree,
    view_root: NodeId,
    selected_index: usize,
    scroll_offset: usize,
    selected_nodes: &'a HashSet<NodeId>,
    theme: &'a Theme,
}

impl<'a> TreeView<'a> {
    pub fn new(
        tree: &'a DiskTree,
        view_root: NodeId,
        selected_index: usize,
        scroll_offset: usize,
        selected_nodes: &'a HashSet<NodeId>,
        theme: &'a Theme,
    ) -> Self {
        Self {
            tree,
            view_root,
            selected_index,
            scroll_offset,
            selected_nodes,
            theme,
        }
    }

    /// Get visible nodes respecting expansion state
    fn visible_nodes(&self) -> Vec<NodeId> {
        self.tree.visible_nodes(self.view_root)
    }

    /// Calculate tree prefixes for each visible node
    fn calculate_prefixes(&self, nodes: &[NodeId]) -> Vec<String> {
        let mut prefixes = Vec::with_capacity(nodes.len());
        let view_root_depth = self.tree.get(self.view_root).map(|n| n.depth).unwrap_or(0);

        for &node_id in nodes {
            let node = match self.tree.get(node_id) {
                Some(n) => n,
                None => {
                    prefixes.push(String::new());
                    continue;
                }
            };

            // Root of view has no prefix
            if node_id == self.view_root {
                prefixes.push(String::new());
                continue;
            }

            let mut prefix = String::new();
            let relative_depth = node.depth.saturating_sub(view_root_depth);

            // Build prefix by walking up the tree
            let path = self.tree.path_to_node(node_id);
            let path: Vec<_> = path
                .into_iter()
                .filter(|&id| {
                    self.tree
                        .get(id)
                        .map(|n| n.depth > view_root_depth)
                        .unwrap_or(false)
                })
                .collect();

            for (i, &ancestor_id) in path.iter().enumerate() {
                if i == path.len() - 1 {
                    // This is the node itself
                    let is_last = self.is_last_sibling(ancestor_id);
                    prefix.push_str(if is_last {
                        TreePrefix::CORNER
                    } else {
                        TreePrefix::TEE
                    });
                } else if relative_depth > 1 {
                    // This is an ancestor
                    let is_last = self.is_last_sibling(ancestor_id);
                    prefix.push_str(if is_last {
                        TreePrefix::BLANK
                    } else {
                        TreePrefix::PIPE
                    });
                }
            }

            prefixes.push(prefix);
        }

        prefixes
    }

    fn is_last_sibling(&self, node_id: NodeId) -> bool {
        let node = match self.tree.get(node_id) {
            Some(n) => n,
            None => return true,
        };

        let parent_id = match node.parent {
            Some(p) => p,
            None => return true,
        };

        let parent = match self.tree.get(parent_id) {
            Some(p) => p,
            None => return true,
        };

        parent.children.last() == Some(&node_id)
    }
}

impl Widget for TreeView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 1 || area.width < 40 {
            return;
        }

        let nodes = self.visible_nodes();
        let prefixes = self.calculate_prefixes(&nodes);
        let total_size = self.tree.get(self.view_root).map(|n| n.size).unwrap_or(1);

        // Column widths
        let bar_width = 24;
        let pct_width = 6;
        let size_width = 10;
        let name_width = area.width as usize - bar_width - pct_width - size_width - 4;

        for (i, (node_id, prefix)) in nodes
            .iter()
            .zip(prefixes.iter())
            .skip(self.scroll_offset)
            .take(area.height as usize)
            .enumerate()
        {
            let y = area.y + i as u16;
            let node = match self.tree.get(*node_id) {
                Some(n) => n,
                None => continue,
            };

            let is_cursor = i + self.scroll_offset == self.selected_index;
            let is_multi_selected = self.selected_nodes.contains(node_id);

            // Three-state: cursor (selection_bg), multi-selected (bg_highlight), normal
            let row_style = if is_cursor {
                Style::default()
                    .bg(self.theme.selection_bg)
                    .fg(self.theme.selection_fg)
            } else if is_multi_selected {
                Style::default()
                    .bg(self.theme.bg_highlight)
                    .fg(self.theme.fg)
            } else {
                Style::default().fg(self.theme.fg)
            };

            // Clear the row
            for x in 0..area.width {
                buf.set_string(area.x + x, y, " ", row_style);
            }

            let mut x = area.x;

            // Selection marker for multi-selected items
            if is_multi_selected && !is_cursor {
                buf.set_string(
                    x,
                    y,
                    "▪ ",
                    Style::default()
                        .bg(self.theme.bg_highlight)
                        .fg(self.theme.purple),
                );
                x += 2;
            } else if is_multi_selected && is_cursor {
                buf.set_string(
                    x,
                    y,
                    "▪ ",
                    Style::default()
                        .bg(self.theme.selection_bg)
                        .fg(self.theme.purple),
                );
                x += 2;
            }

            // Tree prefix
            let prefix_style = if is_cursor {
                row_style.fg(self.theme.selection_fg)
            } else {
                Style::default()
                    .fg(self.theme.border)
                    .bg(if is_multi_selected {
                        self.theme.bg_highlight
                    } else {
                        self.theme.bg
                    })
            };
            buf.set_string(x, y, prefix, prefix_style);
            x += prefix.chars().count() as u16;

            // Icon
            let icon = match node.kind {
                NodeKind::Directory if node.is_expanded => "📂",
                NodeKind::Directory => "📁",
                NodeKind::File => "📄",
                NodeKind::Symlink => "🔗",
                NodeKind::Error => "⚠️",
            };
            let icon_style = if is_cursor {
                row_style
            } else {
                Style::default()
                    .fg(self.theme.icon_color(node.kind.is_directory()))
                    .bg(if is_multi_selected {
                        self.theme.bg_highlight
                    } else {
                        self.theme.bg
                    })
            };
            buf.set_string(x, y, icon, icon_style);
            x += 2; // Icon + space

            // Name
            let name = &node.name;
            let marker_offset = if is_multi_selected { 2 } else { 0 };
            let max_name_len =
                name_width.saturating_sub(prefix.chars().count() + 3 + marker_offset);
            let display_name = if name.len() > max_name_len {
                format!("{}…", &name[..max_name_len.saturating_sub(1)])
            } else {
                name.clone()
            };

            let name_style = if is_cursor {
                row_style.add_modifier(Modifier::BOLD)
            } else if node.kind.is_directory() {
                Style::default()
                    .fg(self.theme.fg)
                    .bg(if is_multi_selected {
                        self.theme.bg_highlight
                    } else {
                        self.theme.bg
                    })
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.theme.fg).bg(if is_multi_selected {
                    self.theme.bg_highlight
                } else {
                    self.theme.bg
                })
            };
            buf.set_string(x, y, &display_name, name_style);

            // Expand indicator for directories
            if node.kind.is_directory() && !node.children.is_empty() {
                let indicator = if node.is_expanded { " ▼" } else { " ▶" };
                let indicator_x = x + display_name.chars().count() as u16;
                let indicator_style = if is_cursor {
                    row_style
                } else {
                    Style::default()
                        .fg(self.theme.fg_muted)
                        .bg(if is_multi_selected {
                            self.theme.bg_highlight
                        } else {
                            self.theme.bg
                        })
                };
                buf.set_string(indicator_x, y, indicator, indicator_style);
            }

            // Size bar (right-aligned section)
            let right_x =
                area.x + area.width - bar_width as u16 - pct_width as u16 - size_width as u16 - 2;

            let percentage = size_percentage(node.size, total_size);
            let bar_color = if is_cursor {
                self.theme.selection_fg
            } else {
                self.theme.size_color(percentage)
            };
            let (bar, _) = render_bar(percentage, bar_width.saturating_sub(2), bar_color);
            buf.set_string(
                right_x,
                y,
                &bar,
                if is_cursor {
                    row_style
                } else {
                    Style::default().fg(bar_color).bg(if is_multi_selected {
                        self.theme.bg_highlight
                    } else {
                        self.theme.bg
                    })
                },
            );

            // Percentage
            let pct_str = format!("{:>5.1}%", percentage);
            let pct_style = if is_cursor {
                row_style
            } else {
                Style::default()
                    .fg(self.theme.fg_dim)
                    .bg(if is_multi_selected {
                        self.theme.bg_highlight
                    } else {
                        self.theme.bg
                    })
            };
            buf.set_string(right_x + bar_width as u16 - 1, y, &pct_str, pct_style);

            // Size
            let size_str = format!("{:>9}", format_size(node.size));
            let size_style = if is_cursor {
                row_style
            } else {
                Style::default()
                    .fg(self.theme.fg_muted)
                    .bg(if is_multi_selected {
                        self.theme.bg_highlight
                    } else {
                        self.theme.bg
                    })
            };
            buf.set_string(
                right_x + bar_width as u16 + pct_width as u16 - 1,
                y,
                &size_str,
                size_style,
            );
        }
    }
}
