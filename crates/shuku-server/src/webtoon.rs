// 普通漫画的页高宽比在 1.4 到 1.5 左右，条漫通常切成高宽比 2 以上的长条。
const WEBTOON_RATIO: f64 = 2.0;

/// 页面高宽比的中位数达到阈值就当条漫。用中位数是为了不受封面和跨页插图影响。
pub(crate) fn looks_like_webtoon(sizes: impl IntoIterator<Item = (u32, u32)>) -> bool {
    let mut ratios: Vec<f64> = sizes
        .into_iter()
        .filter(|(width, _)| *width > 0)
        .map(|(width, height)| f64::from(height) / f64::from(width))
        .collect();
    if ratios.is_empty() {
        return false;
    }
    ratios.sort_by(f64::total_cmp);
    let middle = ratios.len() / 2;
    let median = if ratios.len() % 2 == 1 {
        ratios[middle]
    } else {
        (ratios[middle - 1] + ratios[middle]) / 2.0
    };
    median >= WEBTOON_RATIO
}

#[cfg(test)]
mod tests {
    use super::looks_like_webtoon;

    const MANGA: (u32, u32) = (1200, 1700);
    const STRIP: (u32, u32) = (800, 3200);
    const SPREAD: (u32, u32) = (2400, 1700);

    #[test]
    fn long_strips_are_webtoons_but_manga_with_spreads_is_not() {
        assert!(looks_like_webtoon([MANGA, STRIP, STRIP, STRIP]));
        assert!(!looks_like_webtoon([MANGA, MANGA, SPREAD, MANGA, STRIP]));
    }

    #[test]
    fn even_page_counts_average_the_middle_two_ratios() {
        assert!(!looks_like_webtoon([
            (100, 140),
            (100, 140),
            (100, 220),
            (100, 220)
        ]));
        assert!(looks_like_webtoon([
            (100, 180),
            (100, 220),
            (100, 220),
            (100, 140)
        ]));
    }

    #[test]
    fn empty_books_and_pages_without_a_size_are_not_webtoons() {
        assert!(!looks_like_webtoon([]));
        assert!(!looks_like_webtoon([(0, 0)]));
    }
}
