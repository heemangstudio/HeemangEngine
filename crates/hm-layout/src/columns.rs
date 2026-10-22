//! Balanced (distributed) column regions.
//!
//! A balanced region fills its columns in turn like a newspaper region, but
//! when it ends on a page its lines there are redistributed so the columns
//! come out as even as they can: the region's lines are set again into
//! columns of the smallest height that still holds them all.

use crate::paginate::Flow;
use crate::para::ParaLayout;

/// One line of the region in reading order.
struct Item {
    /// Index into the page's lines.
    at: usize,
    height: i32,
    advance: i32,
    /// Gap from the previous line's advance in the same column.
    gap: i32,
    /// Gap at the top of a column: the paragraph's spacing before for a
    /// first line, nothing for a continuation.
    top_gap: i32,
}

/// Column tops of each item for a column height `h`, or `None` when the
/// items need more columns than `n`.
fn set(items: &[Item], n: usize, h: i32) -> Option<Vec<(usize, i32)>> {
    let mut out = Vec::with_capacity(items.len());
    let mut col = 0;
    let mut cur: Option<i32> = None;
    for it in items {
        let mut y = cur.map_or(it.top_gap, |c| c + it.gap);
        if cur.is_some() && y + it.height > h {
            col += 1;
            if col >= n {
                return None;
            }
            y = it.top_gap;
        }
        out.push((col, y));
        cur = Some(y + it.advance);
    }
    Some(out)
}

/// Balances the lines the current region placed on the current page,
/// starting at page line `first`.
pub(crate) fn balance(flow: &mut Flow, first: usize, layouts: &[ParaLayout]) {
    let n = flow.column_count();
    if n < 2 {
        return;
    }
    let region_y = flow.page_y(0);
    let Some(page) = flow.pages.last() else {
        return;
    };
    let lines = &page.lines[first.min(page.lines.len())..];
    if lines.len() < 2 {
        return;
    }
    let mut items = Vec::with_capacity(lines.len());
    let mut prev: Option<(i32, i32, i32)> = None;
    for (k, pl) in lines.iter().enumerate() {
        let Some(pal) = layouts.get(pl.para as usize) else {
            return;
        };
        let Some(line) = pal.lines.get(pl.line as usize) else {
            return;
        };
        let before = if pl.line == 0 { pal.before } else { 0 };
        let gap = match prev {
            Some((x, y, adv)) if x == pl.x => pl.y - (y + adv),
            _ => before,
        };
        items.push(Item {
            at: first + k,
            height: line.height,
            advance: line.advance(),
            gap: gap.max(0),
            top_gap: before,
        });
        prev = Some((pl.x, pl.y, line.advance()));
    }
    let tallest = items.iter().map(|i| i.height).max().unwrap_or(0);
    let limit = flow.region_height();
    let (mut lo, mut hi) = (tallest, limit);
    if set(&items, n, hi).is_none() {
        return;
    }
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if set(&items, n, mid).is_some() {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    let Some(placement) = set(&items, n, hi) else {
        return;
    };
    let cols: Vec<i32> = (0..n).map(|c| flow.column_x(c)).collect();
    let region_top = flow.region_top();
    let mut fill = vec![0; n];
    let page = flow.pages.last_mut().unwrap();
    for (it, &(col, y)) in items.iter().zip(&placement) {
        let pl = &mut page.lines[it.at];
        pl.x = cols[col];
        pl.y = region_y + y;
        pl.vpos = region_top + y;
        fill[col] = y + it.advance;
    }
    let last = placement.last().map_or(0, |p| p.0);
    flow.set_fill(fill, last);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(h: i32) -> Item {
        Item {
            at: 0,
            height: h,
            advance: h + 100,
            gap: 0,
            top_gap: 0,
        }
    }

    #[test]
    fn six_lines_split_three_and_three() {
        let items: Vec<Item> = (0..6).map(|_| item(1000)).collect();
        // Three lines need 1000 + 1100 * 2 = 3200.
        assert!(set(&items, 2, 3199).is_none());
        let p = set(&items, 2, 3200).unwrap();
        assert_eq!(p.iter().filter(|(c, _)| *c == 0).count(), 3);
        assert_eq!(p[3], (1, 0));
    }
}
