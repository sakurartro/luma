/// Fuzzy subsequence search: returns items whose names contain all query
/// characters in order, ranked by match quality. `lower` must return the
/// precomputed lowercase name — recomputing it per item per query dominated
/// search time (one allocation per name per keystroke).
pub fn fuzzy_search<'a, T>(
    items: &'a [T],
    lower: impl Fn(&'a T) -> &'a [char],
    query: &str,
) -> Vec<&'a T> {
    fuzzy_search_by(items, lower, query, |_| 0)
}

/// fuzzy_search with `bonus(item)` added to each match score (apps: usage_bonus).
pub fn fuzzy_search_by<'a, T>(
    items: &'a [T],
    lower: impl Fn(&'a T) -> &'a [char],
    query: &str,
    bonus: impl Fn(&'a T) -> i32,
) -> Vec<&'a T> {
    if query.is_empty() {
        return items.iter().collect();
    }

    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let mut scored: Vec<(&T, i32)> = Vec::new();

    for item in items {
        if let Some(score) = score_match(lower(item), &query_lower) {
            scored.push((item, score + bonus(item)));
        }
    }

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(item, _)| item).collect()
}

// log2(decayed launch count + 1) capped at 5, +3 if pinned: at most 8, below the
// gap between a prefix/consecutive match and a scattered one, so match quality dominates.
pub fn usage_bonus(score: i64, pinned: bool) -> i32 {
    (score.max(0) + 1).ilog2().min(5) as i32 + if pinned { 3 } else { 0 }
}

/// True when `query` (lowercase) is a fuzzy subsequence of `name` (lowercase);
/// the same rule fuzzy_search uses, for callers that count matches themselves.
pub fn is_match(name: &[char], query: &[char]) -> bool {
    score_match(name, query).is_some()
}

// ponytail: greedy left-to-right subsequence match with simple scoring;
// swap for nucleo if file search needs better ranking
fn score_match(name: &[char], query: &[char]) -> Option<i32> {
    let mut score: i32 = 0;
    let mut qi = 0;
    let mut prev_match: Option<usize> = None;

    for (ni, &nc) in name.iter().enumerate() {
        if qi < query.len() && nc == query[qi] {
            score += 1;

            // Consecutive chars bonus
            if let Some(prev) = prev_match {
                if ni == prev + 1 {
                    score += 3;
                }
            }

            // Start-of-string bonus
            if ni == 0 {
                score += 5;
            // Word-boundary bonus
            } else if matches!(name[ni - 1], ' ' | '-' | '_' | '.') {
                score += 3;
            }

            prev_match = Some(ni);
            qi += 1;
        }
    }

    if qi == query.len() {
        // Prefer shorter names (tighter match)
        score += 10_i32.saturating_sub(name.len() as i32);
        Some(score)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // (lowercase name, decayed launch score, pinned)
    type Entry = (Vec<char>, i64, bool);

    fn app(name: &str, score: i64, pinned: bool) -> Entry {
        (name.to_lowercase().chars().collect(), score, pinned)
    }

    fn ranked(items: &[Entry], query: &str) -> Vec<String> {
        fuzzy_search_by(items, |a| a.0.as_slice(), query, |a| usage_bonus(a.1, a.2))
            .into_iter()
            .map(|a| a.0.iter().collect())
            .collect()
    }

    #[test]
    fn used_app_beats_unused_comparable_match() {
        let items = [app("Files", 0, false), app("Firefox", 20, false)];
        assert_eq!(ranked(&items, "f"), ["firefox", "files"]);
    }

    #[test]
    fn strong_match_beats_heavily_used_scattered_match() {
        // even pinned + max usage can't lift f…i…l over a prefix match
        let items = [app("Fritzing Tool", 1000, true), app("Files", 0, false)];
        assert_eq!(ranked(&items, "fil"), ["files", "fritzing tool"]);
    }

    #[test]
    fn pinned_gets_boost_and_non_matches_stay_hidden() {
        let items = [app("Files", 0, false), app("Firefox", 0, true), app("Terminal", 50, true)];
        assert_eq!(ranked(&items, "f"), ["firefox", "files"]);
    }

    #[test]
    fn plain_search_ranks_by_match_only() {
        let names: Vec<Vec<char>> = ["Firefox", "Files", "Fritzing Tool", "Terminal"]
            .iter()
            .map(|n| n.to_lowercase().chars().collect())
            .collect();
        let hits: Vec<String> = fuzzy_search(&names, |n| n.as_slice(), "f")
            .into_iter()
            .map(|n| n.iter().collect())
            .collect();
        assert_eq!(hits, ["files", "firefox", "fritzing tool"]);
        assert_eq!(fuzzy_search(&names, |n| n.as_slice(), "").len(), 4);
    }
}
