//! A read-only table of text cells (events, table rows, electrodes): virtualized, sortable by
//! clicking a header (numbers sort as numbers).

use std::cmp::Ordering;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::{div, px, App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window};

/// Columns and rows to show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableData {
    /// (name, numeric: right-aligned and sorted as numbers)
    pub columns: Vec<(String, bool)>,
    pub rows: Vec<Vec<String>>,
}

pub struct TextTable {
    data: TableData,
    /// Row order shown (indices into `data.rows`).
    order: Vec<usize>,
}

impl TextTable {
    pub fn new(data: TableData) -> Self {
        let order = (0..data.rows.len()).collect();
        Self { data, order }
    }

    pub fn data(&self) -> &TableData {
        &self.data
    }
}

fn number(s: &str) -> f64 {
    s.trim().parse().unwrap_or(f64::NAN)
}

impl TableDelegate for TextTable {
    fn columns_count(&self, _: &App) -> usize {
        self.data.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.data.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let (name, numeric) = &self.data.columns[col_ix];
        let width = if *numeric { px(120.) } else { px(220.) };
        let c = Column::new(SharedString::from(format!("c{col_ix}")), SharedString::from(name.clone())).width(width).sortable();
        if *numeric { c.text_right() } else { c }
    }

    fn perform_sort(&mut self, col_ix: usize, sort: ColumnSort, _: &mut Window, _: &mut Context<TableState<Self>>) {
        let numeric = self.data.columns[col_ix].1;
        let rows = &self.data.rows;
        let cmp = |a: &usize, b: &usize| {
            let (x, y) = (&rows[*a][col_ix], &rows[*b][col_ix]);
            if numeric { number(x).partial_cmp(&number(y)).unwrap_or(Ordering::Equal) } else { x.cmp(y) }
        };
        match sort {
            ColumnSort::Default => self.order = (0..rows.len()).collect(),
            ColumnSort::Ascending => self.order.sort_by(cmp),
            ColumnSort::Descending => self.order.sort_by(|a, b| cmp(b, a)),
        }
    }

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _: &mut Window, _: &mut Context<TableState<Self>>) -> impl IntoElement {
        let row = self.order.get(row_ix).copied().unwrap_or(row_ix);
        div().text_sm().child(self.data.rows.get(row).and_then(|r| r.get(col_ix)).cloned().unwrap_or_default())
    }
}
