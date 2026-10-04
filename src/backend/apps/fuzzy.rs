use super::apps_search::App;

/// Fuzzy subsequence search: returns apps whose names contain all query
/// characters in order, ranked by match quality.
pub fn fuzzy_search<'a>(apps: &'a [App], query: &str) -> Vec<&'a App> {
    if query.is_empty() {
        return apps.iter().collect();
    }

    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let mut scored: Vec<(&App, i32)> = Vec::new();

    for app in apps {
        let name_lower: Vec<char> = app.app_name.to_lowercase().chars().collect();
        if let Some(score) = score_match(&name_lower, &query_lower) {
            scored.push((app, score));
        }
    }

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(app, _)| app).collect()
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
