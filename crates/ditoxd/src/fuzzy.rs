pub fn score(haystack: &[u8], needle: &[u8]) -> i64 {
    let needle = needle.trim_ascii();
    if haystack.is_empty() || needle.is_empty() {
        return 0;
    }
    score_mode(haystack, needle, false).max(score_mode(haystack, needle, true))
}

fn score_mode(haystack: &[u8], needle: &[u8], prefer_boundaries: bool) -> i64 {
    let mut search_start = 0usize;
    let mut previous: Option<usize> = None;
    let mut first: Option<usize> = None;
    let mut last: Option<usize> = None;
    let mut run = 0i64;
    let mut longest_run = 0i64;
    let mut boundary_matches = 0i64;
    let mut score = 0i64;
    let needle_len = needle.len() as i64;

    for byte in needle {
        let wanted = byte.to_ascii_lowercase();
        let Some(index) = find_match(haystack, wanted, search_start, previous, prefer_boundaries)
        else {
            return 0;
        };
        first.get_or_insert(index);

        run = if previous.is_some_and(|before| index == before + 1) {
            run + 1
        } else {
            1
        };
        longest_run = longest_run.max(run);

        score += 10 + run * 12;
        let on_boundary = is_boundary(haystack, index);
        if on_boundary {
            score += 18;
            boundary_matches += 1;
        }
        if index < 8 {
            score += (8 - index) as i64;
        }
        if let Some(before) = previous {
            let gap = (index - before - 1) as i64;
            score -= if on_boundary {
                (gap * 2).min(16)
            } else {
                (gap * 5).min(32)
            };
        }

        previous = Some(index);
        last = Some(index);
        search_start = index + 1;
    }

    if let Some(index) = first {
        score -= (index as i64).min(24);
    }
    if let (Some(start), Some(end)) = (first, last) {
        let span = (end - start + 1) as i64;
        score -= ((span - needle_len) * 2).min(40);
    }
    if boundary_matches == needle_len {
        score += 40 + needle_len * 4;
    }
    if longest_run == needle_len {
        score += 80;
    }
    score -= ((haystack.len() / 24) as i64).min(12);
    score.max(1)
}

fn find_match(
    haystack: &[u8],
    wanted: u8,
    start: usize,
    previous: Option<usize>,
    prefer_boundaries: bool,
) -> Option<usize> {
    let mut first = None;
    let mut first_boundary = None;
    for index in start..haystack.len() {
        if haystack[index].to_ascii_lowercase() != wanted {
            continue;
        }
        if previous.is_some_and(|before| index == before + 1) {
            return Some(index);
        }
        first.get_or_insert(index);
        if prefer_boundaries && first_boundary.is_none() && is_boundary(haystack, index) {
            first_boundary = Some(index);
        }
    }
    first_boundary.or(first)
}

fn is_boundary(haystack: &[u8], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let before = haystack[index - 1];
    matches!(
        before,
        b' ' | b'\t' | b'\n' | b'\r' | b'-' | b'_' | b'/' | b'\\' | b'.' | b':'
    ) || (before.is_ascii_lowercase() && haystack[index].is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn contiguous_acronym_path_and_camel_case_matches_rank_higher() {
        assert!(score(b"network latency error", b"nle") > 0);
        assert_eq!(score(b"totally separate", b"nle"), 0);
        assert!(score(b"needle", b"nee") > score(b"n-e-e", b"nee"));
        assert!(score(b"network latency error", b"nle") > score(b"annular low effect", b"nle"));
        assert!(
            score(b"src/components/PreviewPane.tsx", b"ppt")
                > score(b"support pipeline target", b"ppt")
        );
    }

    #[test]
    fn empty_input_scores_nothing() {
        assert_eq!(score(b"", b"a"), 0);
        assert_eq!(score(b"abc", b"  "), 0);
    }
}
