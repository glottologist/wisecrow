use crate::errors::WisecrowError;
use crate::grammar::pdf::{ExampleSentence, GrammarSection};

#[derive(Debug, Clone)]
pub struct ClozeQuiz {
    pub sentence_with_blank: String,
    pub answer: String,
    pub hint: Option<String>,
    pub rule_id: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct MultipleChoiceQuiz {
    pub question: String,
    pub options: Vec<String>,
    pub correct_index: usize,
    pub rule_id: Option<i32>,
}

pub struct QuizGenerator;

impl QuizGenerator {
    /// Generates cloze deletion quizzes from example sentences.
    ///
    /// Each sentence produces one quiz by removing a word (preferring
    /// longer words as they tend to be more meaningful).
    #[must_use]
    pub fn cloze_from_examples(examples: &[ExampleSentence]) -> Vec<ClozeQuiz> {
        examples.iter().filter_map(Self::make_cloze).collect()
    }

    /// Generates multiple-choice quizzes from grammar rules using example
    /// sentences as distractors.
    ///
    /// # Errors
    ///
    /// Returns an error if there are insufficient rules to generate quizzes.
    pub fn multiple_choice_from_rules(
        sections: &[GrammarSection],
    ) -> Result<Vec<MultipleChoiceQuiz>, WisecrowError> {
        const QUESTION_TEXT: &str = "Which of the following is a correct grammar rule?";

        let all_rules: Vec<&str> = sections
            .iter()
            .flat_map(|s| s.rules.iter().map(String::as_str))
            .collect();

        if all_rules.len() < 2 {
            return Err(WisecrowError::QuizGenerationError(
                "Need at least 2 rules to generate multiple-choice quizzes".to_owned(),
            ));
        }

        let mut quizzes = Vec::new();
        let question = QUESTION_TEXT.to_owned();

        for (idx, &rule) in all_rules.iter().enumerate() {
            let distractors: Vec<String> = all_rules
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != idx)
                .take(3)
                .map(|(_, r)| (*r).to_owned())
                .collect();

            if distractors.is_empty() {
                continue;
            }

            let mut options = vec![rule.to_owned()];
            options.extend(distractors);

            quizzes.push(MultipleChoiceQuiz {
                question: question.clone(), // clone: reusing pre-allocated question string
                options,
                correct_index: 0,
                rule_id: None,
            });
        }

        Ok(quizzes)
    }

    fn make_cloze(example: &ExampleSentence) -> Option<ClozeQuiz> {
        let words: Vec<&str> = example.text.split_whitespace().collect();

        if words.len() < 3 {
            return None;
        }

        let (target_idx, target_word) = words
            .iter()
            .enumerate()
            .filter(|(_, w)| w.len() >= 3)
            .max_by_key(|(_, w)| w.len())?;

        let blanked: Vec<&str> = words
            .iter()
            .enumerate()
            .map(|(i, w)| if i == target_idx { "____" } else { w })
            .collect();

        let first_char = target_word.chars().next()?;

        Some(ClozeQuiz {
            sentence_with_blank: blanked.join(" "),
            answer: (*target_word).to_owned(),
            hint: Some(format!("Starts with '{first_char}'")),
            rule_id: None,
        })
    }
}

/// Shuffles quiz options deterministically based on a seed, preserving
/// the correct answer index.
#[must_use]
pub fn shuffle_options(quiz: &MultipleChoiceQuiz, seed: usize) -> MultipleChoiceQuiz {
    let len = quiz.options.len();
    if len <= 1 {
        return quiz.clone(); // clone: quiz is small, creating shuffled copy
    }

    let mut indices: Vec<usize> = (0..len).collect();

    for i in (1..len).rev() {
        // Knuth multiplicative hash (golden ratio constant 2654435761)
        let j = seed.wrapping_add(i).wrapping_mul(2654435761) % i.saturating_add(1);
        indices.swap(i, j);
    }

    let options: Vec<String> = indices
        .iter()
        .map(|&i| quiz.options[i].clone()) // clone: building new shuffled vec
        .collect();

    let correct_index = indices
        .iter()
        .position(|&i| i == quiz.correct_index)
        .unwrap_or(0);

    MultipleChoiceQuiz {
        question: quiz.question.clone(), // clone: building new owned struct
        options,
        correct_index,
        rule_id: quiz.rule_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::pdf::ExampleSentence;
    use proptest::prelude::*;
    use rstest::rstest;

    #[rstest]
    #[case(
        "Yo hablo espa\u{f1}ol",
        Some("I speak Spanish"),
        1,
        "Yo hablo ____",
        "espa\u{f1}ol"
    )]
    #[case("Si no", None, 0, "", "")]
    #[case("The cat sat quietly", None, 1, "The cat sat ____", "quietly")]
    fn cloze_generation(
        #[case] text: &str,
        #[case] translation: Option<&str>,
        #[case] expected_count: usize,
        #[case] expected_blank: &str,
        #[case] expected_answer: &str,
    ) {
        let examples = vec![ExampleSentence {
            text: text.to_owned(),
            translation: translation.map(str::to_owned),
        }];
        let quizzes = QuizGenerator::cloze_from_examples(&examples);
        assert_eq!(quizzes.len(), expected_count);
        if expected_count > 0 {
            assert_eq!(quizzes[0].sentence_with_blank, expected_blank);
            assert_eq!(quizzes[0].answer, expected_answer);
        }
    }

    #[test]
    fn multiple_choice_needs_minimum_rules() {
        let sections = vec![GrammarSection {
            title: None,
            rules: vec!["Only one rule".to_owned()],
            examples: vec![],
        }];

        let result = QuizGenerator::multiple_choice_from_rules(&sections);
        assert!(result.is_err());
    }

    #[test]
    fn multiple_choice_generates_from_rules() {
        let sections = vec![GrammarSection {
            title: Some("Verbs".to_owned()),
            rules: vec![
                "Regular verbs end in -ar".to_owned(),
                "Irregular verbs must be memorized".to_owned(),
                "Reflexive verbs use se".to_owned(),
            ],
            examples: vec![],
        }];

        let quizzes = QuizGenerator::multiple_choice_from_rules(&sections).unwrap();
        assert_eq!(quizzes.len(), 3);
        for quiz in &quizzes {
            assert!(!quiz.options.is_empty());
            assert!(quiz.correct_index < quiz.options.len());
        }
    }

    #[rstest]
    #[case(0)]
    #[case(1)]
    fn shuffle_with_few_options_is_identity(#[case] n: usize) {
        let options: Vec<String> = (0..n).map(|i| format!("opt{i}")).collect();
        let quiz = MultipleChoiceQuiz {
            question: "test".to_owned(),
            options: options.clone(), // clone: need original for comparison
            correct_index: 0,
            rule_id: None,
        };
        let result = shuffle_options(&quiz, 42);
        assert_eq!(result.options, options);
    }

    proptest! {
        #[test]
        fn shuffle_preserves_correct_answer(seed in 0usize..10000) {
            let quiz = MultipleChoiceQuiz {
                question: "Test?".to_owned(),
                options: vec![
                    "A".to_owned(),
                    "B".to_owned(),
                    "C".to_owned(),
                    "D".to_owned(),
                ],
                correct_index: 0,
                rule_id: None,
            };
            let shuffled = shuffle_options(&quiz, seed);
            prop_assert_eq!(&shuffled.options[shuffled.correct_index], "A");
        }
    }
}
