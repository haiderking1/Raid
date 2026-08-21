#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

pub fn model_search_text(id: &str, provider: &str, name: &str) -> String {
    format!("{provider} {provider}/{id} {provider} {id} {name}")
}

pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let query_lower = query.to_ascii_lowercase();
    let text_lower = text.to_ascii_lowercase();

    let primary = match_query(&query_lower, &text_lower);
    if primary.matches {
        return primary;
    }

    let swapped = swapped_alpha_numeric_query(&query_lower);
    let Some(swapped) = swapped else {
        return primary;
    };

    let swapped_match = match_query(&swapped, &text_lower);
    if swapped_match.matches {
        return FuzzyMatch {
            matches: true,
            score: swapped_match.score + 5.0,
        };
    }
    primary
}

pub fn fuzzy_filter_indices<T>(
    items: &[T],
    query: &str,
    get_text: impl Fn(&T) -> String,
) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..items.len()).collect();
    }

    let tokens: Vec<String> = query
        .split(|ch: char| ch.is_whitespace() || ch == '/')
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if tokens.is_empty() {
        return (0..items.len()).collect();
    }

    let mut matches = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let text = get_text(item);
        let mut total_score = 0.0;
        let mut all_match = true;
        for token in &tokens {
            let result = fuzzy_match(token, &text);
            if result.matches {
                total_score += result.score;
            } else {
                all_match = false;
                break;
            }
        }
        if all_match {
            matches.push((index, total_score));
        }
    }

    matches.sort_by(|(left, left_score), (right, right_score)| {
        left_score
            .total_cmp(right_score)
            .then_with(|| left.cmp(right))
    });
    matches.into_iter().map(|(index, _)| index).collect()
}

fn match_query(query: &str, text: &str) -> FuzzyMatch {
    if query.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }
    if query.chars().count() > text.chars().count() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    let query_chars: Vec<char> = query.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    let mut query_index = 0usize;
    let mut score = 0.0;
    let mut last_match_index: isize = -1;
    let mut consecutive_matches = 0usize;

    for (index, ch) in text_chars.iter().enumerate() {
        if query_index >= query_chars.len() {
            break;
        }
        if *ch != query_chars[query_index] {
            continue;
        }

        let is_word_boundary = index == 0
            || text_chars
                .get(index - 1)
                .is_some_and(|previous| is_boundary_char(*previous));

        if last_match_index == index as isize - 1 {
            consecutive_matches += 1;
            score -= (consecutive_matches as f64) * 5.0;
        } else {
            consecutive_matches = 0;
            if last_match_index >= 0 {
                score += ((index as isize - last_match_index - 1) as f64) * 2.0;
            }
        }

        if is_word_boundary {
            score -= 10.0;
        }

        score += index as f64 * 0.1;
        last_match_index = index as isize;
        query_index += 1;
    }

    if query_index < query_chars.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    if query == text {
        score -= 100.0;
    }

    FuzzyMatch {
        matches: true,
        score,
    }
}

fn is_boundary_char(ch: char) -> bool {
    matches!(ch, ' ' | '-' | '_' | '.' | '/' | ':')
}

fn swapped_alpha_numeric_query(query: &str) -> Option<String> {
    let letters_digits = split_alpha_numeric(query);
    let digits_letters = split_numeric_alpha(query);
    letters_digits
        .or(digits_letters)
        .filter(|swapped| swapped != query)
}

fn split_alpha_numeric(query: &str) -> Option<String> {
    let mut letters = String::new();
    let mut digits = String::new();
    let mut mode = None;
    for ch in query.chars() {
        if ch.is_ascii_alphabetic() {
            if matches!(mode, Some('d')) {
                return None;
            }
            mode = Some('a');
            letters.push(ch);
        } else if ch.is_ascii_digit() {
            if letters.is_empty() {
                return None;
            }
            mode = Some('d');
            digits.push(ch);
        } else {
            return None;
        }
    }
    if letters.is_empty() || digits.is_empty() {
        return None;
    }
    Some(format!("{digits}{letters}"))
}

fn split_numeric_alpha(query: &str) -> Option<String> {
    let mut digits = String::new();
    let mut letters = String::new();
    let mut mode = None;
    for ch in query.chars() {
        if ch.is_ascii_digit() {
            if matches!(mode, Some('a')) {
                return None;
            }
            mode = Some('d');
            digits.push(ch);
        } else if ch.is_ascii_alphabetic() {
            if digits.is_empty() {
                return None;
            }
            mode = Some('a');
            letters.push(ch);
        } else {
            return None;
        }
    }
    if digits.is_empty() || letters.is_empty() {
        return None;
    }
    Some(format!("{letters}{digits}"))
}

#[cfg(test)]
mod tests {
    use super::{fuzzy_filter_indices, fuzzy_match, model_search_text};

    #[test]
    fn empty_query_matches_everything() {
        let result = fuzzy_match("", "anything");
        assert!(result.matches);
        assert_eq!(result.score, 0.0);
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(fuzzy_match("abc", "aXbXc").matches);
        assert!(!fuzzy_match("abc", "cba").matches);
    }

    #[test]
    fn consecutive_matches_score_better_than_scattered_matches() {
        let consecutive = fuzzy_match("foo", "foobar");
        let scattered = fuzzy_match("foo", "f_o_o_bar");
        assert!(consecutive.matches);
        assert!(scattered.matches);
        assert!(consecutive.score < scattered.score);
    }

    #[test]
    fn word_boundary_matches_score_better() {
        let at_boundary = fuzzy_match("fb", "foo-bar");
        let not_at_boundary = fuzzy_match("fb", "afbx");
        assert!(at_boundary.matches);
        assert!(not_at_boundary.matches);
        assert!(at_boundary.score < not_at_boundary.score);
    }

    #[test]
    fn matches_swapped_alpha_numeric_tokens() {
        assert!(fuzzy_match("codex52", "gpt-5.2-codex").matches);
    }

    #[test]
    fn fuzzy_filter_sorts_by_match_quality() {
        let items = ["a_p_p", "app", "application"];
        let filtered = fuzzy_filter_indices(&items, "app", |item| item.to_string());
        assert_eq!(filtered.first().copied(), Some(1));
    }

    #[test]
    fn model_search_ranks_luna_before_plus_for_lu() {
        #[derive(Clone)]
        struct Model {
            id: &'static str,
            name: &'static str,
        }
        let models = [
            Model {
                id: "qwen3.7-plus",
                name: "Qwen3.7 Plus",
            },
            Model {
                id: "qwen3.6-plus",
                name: "Qwen3.6 Plus",
            },
            Model {
                id: "gpt-5.6-luna",
                name: "GPT-5.6 Luna",
            },
            Model {
                id: "ox-alpha-free",
                name: "Ox Alpha Free",
            },
        ];
        let filtered = fuzzy_filter_indices(&models, "lu", |model| {
            model_search_text(model.id, "opencode-go", model.name)
        });
        assert_eq!(filtered, vec![2, 0, 1]);
        assert!(!filtered.contains(&3));
    }
}
