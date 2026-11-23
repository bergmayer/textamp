//! Cell-aligned terminal rules and blocks, independent of font line-height padding.
use gpui_kit::*;

fn block_fill(text: &str) -> Option<([u8; 8], f32)> {
    let mut characters = text.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    let mut rows = [0; 8];
    let mut opacity = 1.;
    match character {
        '▀' => rows[..4].fill(255),
        '▁'..='█' => {
            let height = character as usize - '▁' as usize + 1;
            rows[8 - height..].fill(255);
        }
        '▉'..='▏' => {
            let width = 7 - (character as u32 - '▉' as u32);
            rows.fill(255 << (8 - width));
        }
        '▐' => rows.fill(15),
        '░' | '▒' | '▓' => {
            rows.fill(255);
            opacity = (character as u32 - '░' as u32 + 1) as f32 / 4.;
        }
        '▔' => rows[0] = 255,
        '▕' => rows.fill(1),
        '▖'..='▟' => {
            let quadrants =
                [4, 8, 1, 13, 9, 7, 11, 2, 6, 14][(character as u32 - '▖' as u32) as usize];
            for (y, row) in rows.iter_mut().enumerate() {
                let shift = if y < 4 { 0 } else { 2 };
                *row = if quadrants & (1 << shift) != 0 {
                    240
                } else {
                    0
                } | if quadrants & (2 << shift) != 0 { 15 } else { 0 };
            }
        }
        _ => return None,
    }
    Some((rows, opacity))
}

fn edges(text: &str) -> Option<[u8; 4]> {
    let mut chars = text.chars();
    let character = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(match character {
        '─' => [0, 1, 0, 1],
        '│' => [1, 0, 1, 0],
        '┌' | '╭' => [0, 1, 1, 0],
        '┐' | '╮' => [0, 0, 1, 1],
        '└' | '╰' => [1, 1, 0, 0],
        '┘' | '╯' => [1, 0, 0, 1],
        '├' => [1, 1, 1, 0],
        '┤' => [1, 0, 1, 1],
        '┬' => [0, 1, 1, 1],
        '┴' => [1, 1, 0, 1],
        '┼' => [1, 1, 1, 1],
        '━' => [0, 3, 0, 3],
        '┃' => [3, 0, 3, 0],
        '┏' => [0, 3, 3, 0],
        '┓' => [0, 0, 3, 3],
        '┗' => [3, 3, 0, 0],
        '┛' => [3, 0, 0, 3],
        '┣' => [3, 3, 3, 0],
        '┫' => [3, 0, 3, 3],
        '┳' => [0, 3, 3, 3],
        '┻' => [3, 3, 0, 3],
        '╋' => [3, 3, 3, 3],
        '═' => [0, 2, 0, 2],
        '║' => [2, 0, 2, 0],
        '╔' => [0, 2, 2, 0],
        '╗' => [0, 0, 2, 2],
        '╚' => [2, 2, 0, 0],
        '╝' => [2, 0, 0, 2],
        '╠' => [2, 2, 2, 0],
        '╣' => [2, 0, 2, 2],
        '╦' => [0, 2, 2, 2],
        '╩' => [2, 2, 0, 2],
        '╬' => [2, 2, 2, 2],
        _ => return None,
    })
}

/// Whether this glyph has a continuous cell-aligned shape representation.
pub(super) fn supports(text: &str) -> bool {
    edges(text).is_some() || block_fill(text).is_some()
}

/// Paint rules, scrollbar blocks and fractional artwork across the complete cell.
pub(super) fn paint(text: &str, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) -> bool {
    if let Some((rows, opacity)) = block_fill(text) {
        let mut start = 0;
        while start < rows.len() {
            let bits = rows[start];
            let end = rows[start..]
                .iter()
                .position(|row| *row != bits)
                .map_or(rows.len(), |length| start + length);
            if bits != 0 {
                window.paint_quad(fill(
                    Bounds::new(
                        bounds.origin
                            + point(
                                bounds.size.width * (bits.leading_zeros() as f32 / 8.),
                                bounds.size.height * (start as f32 / 8.),
                            ),
                        size(
                            bounds.size.width * (bits.count_ones() as f32 / 8.),
                            bounds.size.height * ((end - start) as f32 / 8.),
                        ),
                    ),
                    color.opacity(opacity),
                ));
            }
            start = end;
        }
        return true;
    }
    let Some(edges) = edges(text) else {
        return false;
    };
    let center = bounds.center();
    let thickness = px(1.);
    let gap = thickness * 2.;
    for (direction, weight) in edges.into_iter().enumerate() {
        let offsets = if weight == 2 {
            &[-1., 1.][..]
        } else if weight == 1 || weight == 3 {
            &[0.][..]
        } else {
            &[][..]
        };
        for offset in offsets {
            let thickness = if weight == 3 { px(1.5) } else { thickness };
            let shift = gap * (*offset);
            let rule = match direction {
                0 => Bounds::new(
                    point(center.x + shift, bounds.top()),
                    size(thickness, center.y - bounds.top() + gap),
                ),
                1 => Bounds::new(
                    point(center.x - gap, center.y + shift),
                    size(bounds.right() - center.x + gap, thickness),
                ),
                2 => Bounds::new(
                    point(center.x + shift, center.y - gap),
                    size(thickness, bounds.bottom() - center.y + gap),
                ),
                _ => Bounds::new(
                    point(bounds.left(), center.y + shift),
                    size(center.x - bounds.left() + gap, thickness),
                ),
            };
            window.paint_quad(fill(rule, color));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::block_fill;
    #[test]
    fn terminal_blocks_fill_scrollbars_and_complementary_artwork_cells() {
        assert_eq!(block_fill("█"), Some(([255; 8], 1.)));
        let (upper, _) = block_fill("▀").unwrap();
        let (lower, _) = block_fill("▄").unwrap();
        for (upper, lower) in upper.into_iter().zip(lower) {
            assert_eq!(upper | lower, 255);
            assert_eq!(upper & lower, 0);
        }
        assert_eq!(block_fill("▏"), Some(([128; 8], 1.)));
        assert_eq!(block_fill("▕"), Some(([1; 8], 1.)));
        assert!(block_fill("e\u{301}").is_none());
    }
}
