//! Where each part of the picker goes, in device pixels from the window's top left.

use crate::config::{Config, Length};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn inset(self, by: f32) -> Rect {
        Rect {
            left: self.left + by,
            top: self.top + by,
            right: self.right - by,
            bottom: self.bottom - by,
        }
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        (self.left..self.right).contains(&x) && (self.top..self.bottom).contains(&y)
    }
}

/// One slot of the grid. `label` sits beside `icon`, so a row without an icon starts its label at `icon.left`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub area: Rect,
    pub icon: Rect,
    pub label: Rect,
}

#[derive(Debug, PartialEq)]
pub struct Layout {
    /// Whole pixels, since the window's size is.
    pub width: f32,
    pub height: f32,
    pub border: f32,
    pub input: Rect,
    pub prompt: Rect,
    pub entry: Rect,
    pub list: Rect,
    /// Column by column, as the matches fill them.
    pub cells: Vec<Cell>,
}

impl Layout {
    pub fn cell_at(&self, x: f32, y: f32) -> Option<usize> {
        self.cells.iter().position(|cell| cell.area.contains(x, y))
    }
}

/// Lays out `config` for a font `em` px tall on a monitor at `scale` (1.0 at 96 DPI). `line` is the height of a line
/// of text and `prompt` the width of the prompt, both as DirectWrite measured them.
pub fn measure(config: &Config, em: f32, scale: f32, line: f32, prompt: f32) -> Layout {
    let px = |length: Length| length.px(em, scale);
    let (input, list, element) = (&config.input, &config.list, &config.element);
    let border = px(config.window.border);
    let width = px(config.window.width).ceil();
    let margin = px(input.margin);
    let bar = Rect {
        left: border + margin,
        top: border + margin,
        right: width - border - margin,
        bottom: border + margin + 2.0 * px(input.padding) + line,
    };
    let text = bar.inset(px(input.padding));
    let prompt = Rect {
        right: text.left + prompt,
        ..text
    };
    let entry = Rect {
        left: prompt.right + px(input.prompt_gap),
        ..text
    };

    let (columns, lines) = (list.columns.get(), list.lines.get());
    let (padding, spacing) = (px(list.padding), px(list.spacing));
    let (inner, icon) = (px(element.padding), px(element.icon));
    let cell_height = 2.0 * inner + icon.max(line);
    let top = bar.bottom + margin;
    let grid = Rect {
        left: border + padding,
        top: top + padding,
        right: width - border - padding,
        bottom: top + padding + lines as f32 * (cell_height + spacing) - spacing,
    };
    let cell_width = (grid.right - grid.left + spacing) / columns as f32 - spacing;
    let height = (grid.bottom + padding + border).ceil();
    let cells = (0..columns * lines)
        .map(|slot| {
            let left = grid.left + (slot / lines) as f32 * (cell_width + spacing);
            let top = grid.top + (slot % lines) as f32 * (cell_height + spacing);
            let area = Rect {
                left,
                top,
                right: left + cell_width,
                bottom: top + cell_height,
            };
            let icon_top = top + (cell_height - icon) / 2.0;
            let icon = Rect {
                left: left + inner,
                top: icon_top,
                right: left + inner + icon,
                bottom: icon_top + icon,
            };
            let label = Rect {
                left: icon.right + px(element.gap),
                ..area.inset(inner)
            };
            Cell { area, icon, label }
        })
        .collect();
    Layout {
        width,
        height,
        border,
        input: bar,
        prompt,
        entry,
        list: Rect {
            left: border,
            top,
            right: width - border,
            bottom: height - border,
        },
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r##"
        [font]
        family = "Test"
        size = 10
        [window]
        width = "200px"
        background = "#000000"
        border = "2px"
        border_color = "#ffffff"
        radius = "1em"
        [input]
        margin = "10px"
        padding = "1em"
        radius = "1em"
        background = "#000000"
        color = "#ffffff"
        prompt = ">"
        prompt_font = "Test"
        prompt_gap = "4px"
        placeholder = "Search"
        placeholder_color = "#ffffff"
        [list]
        background = "#000000"
        padding = "6px"
        spacing = "4px"
        columns = 2
        lines = 3
        [element]
        padding = "5px"
        radius = "1em"
        color = "#ffffff"
        selected = "#ffffff"
        icon = "20px"
        gap = "3px"
    "##;

    fn layout(scale: f32) -> Result<Layout, toml::de::Error> {
        let config: Config = toml::from_str(CONFIG)?;
        Ok(measure(&config, 10.0, scale, 16.0, 8.0))
    }

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn the_input_bar_holds_the_prompt_then_the_entry() -> Result<(), toml::de::Error> {
        let layout = layout(1.0)?;
        assert_eq!(layout.input, rect(12.0, 12.0, 188.0, 48.0));
        assert_eq!(layout.prompt, rect(22.0, 22.0, 30.0, 38.0));
        assert_eq!(layout.entry, rect(34.0, 22.0, 178.0, 38.0));
        Ok(())
    }

    #[test]
    fn the_grid_fills_column_by_column() -> Result<(), toml::de::Error> {
        let layout = layout(1.0)?;
        let areas: Vec<Rect> = layout.cells.iter().map(|cell| cell.area).collect();
        // The list starts a margin below the bar, cells are 30px tall and (184 - 4) / 2 = 90px wide.
        assert_eq!(
            areas,
            [
                rect(8.0, 64.0, 98.0, 94.0),
                rect(8.0, 98.0, 98.0, 128.0),
                rect(8.0, 132.0, 98.0, 162.0),
                rect(102.0, 64.0, 192.0, 94.0),
                rect(102.0, 98.0, 192.0, 128.0),
                rect(102.0, 132.0, 192.0, 162.0),
            ]
        );
        assert_eq!((layout.width, layout.height), (200.0, 170.0));
        assert_eq!(layout.list, rect(2.0, 58.0, 198.0, 168.0));
        Ok(())
    }

    #[test]
    fn the_label_follows_the_centered_icon() -> Result<(), toml::de::Error> {
        let cell = layout(1.0)?.cells.first().copied();
        assert_eq!(
            cell.map(|cell| cell.icon),
            Some(rect(13.0, 69.0, 33.0, 89.0))
        );
        assert_eq!(
            cell.map(|cell| cell.label),
            Some(rect(36.0, 69.0, 93.0, 89.0))
        );
        Ok(())
    }

    #[test]
    fn px_lengths_scale_and_the_size_rounds_up() -> Result<(), toml::de::Error> {
        let layout = layout(1.25)?;
        assert_eq!(layout.width, 250.0);
        assert_eq!(layout.border, 2.5);
        assert_eq!(layout.height, 204.0);
        Ok(())
    }

    #[test]
    fn cell_at_finds_the_cell_under_a_point() -> Result<(), toml::de::Error> {
        let layout = layout(1.0)?;
        assert_eq!(layout.cell_at(110.0, 100.0), Some(4));
        assert_eq!(layout.cell_at(100.0, 100.0), None);
        assert_eq!(layout.cell_at(100.0, 20.0), None);
        Ok(())
    }
}
