use wisecrow::grammar::paradigms::{
    native_face, paradigm_prompt, slots_for, validate_paradigm, ParadigmPromptSlot, ParadigmSlot,
    FORM_CHARS_MAX, FORM_WORDS_MAX, NATIVE_FACE_CHARS_MAX, NATIVE_WORDS_MAX, PARADIGM_VERSION,
};
use wisecrow::lang::MAX_WORD_CHARS;

fn ids(language: &str) -> Vec<&'static str> {
    slots_for(language)
        .expect(language)
        .iter()
        .map(|slot| slot.id)
        .collect()
}

fn label_at(language: &str, index: usize) -> &'static str {
    slots_for(language).expect(language)[index].label
}

fn filled(language: &str) -> Vec<ParadigmSlot> {
    slots_for(language)
        .expect(language)
        .iter()
        .map(|slot| ParadigmSlot {
            slot: slot.id.to_owned(),
            form: "parle".to_owned(),
            native: "I speak".to_owned(),
        })
        .collect()
}

fn words(n: usize) -> String {
    std::iter::repeat_n("we", n).collect::<Vec<_>>().join(" ")
}

#[test]
fn slot_table_covers_the_four_languages_without_repeats() {
    assert_eq!(PARADIGM_VERSION, 1);
    let cases = [("fr", 39), ("it", 39), ("es", 39), ("gd", 7)];
    for (language, count) in cases {
        let slots = slots_for(language).expect(language);
        assert_eq!(slots.len(), count, "{language}");
        let mut seen = std::collections::HashSet::new();
        for slot in slots {
            assert!(seen.insert(slot.id), "duplicate {language} {}", slot.id);
            assert!(!slot.id.is_empty() && slot.id.len() <= 32, "{}", slot.id);
            assert!(!slot.label.is_empty(), "{}", slot.id);
            assert!(!slot.id.chars().any(char::is_whitespace), "{}", slot.id);
        }
    }
    for language in ["fr", "it", "es"] {
        let imperative: Vec<_> = ids(language)
            .into_iter()
            .filter(|id| id.starts_with("imp."))
            .collect();
        assert_eq!(imperative, ["imp.2s", "imp.1p", "imp.2p"], "{language}");
    }
    assert_eq!(
        ids("gd"),
        ["past", "fut", "cond.1s", "cond.1p", "imp.2s", "imp.2p", "vn"]
    );
}

#[test]
fn romance_slots_follow_the_teaching_order() {
    let french = ids("fr");
    assert_eq!(
        &french[..6],
        ["pres.1s", "pres.2s", "pres.3s", "pres.1p", "pres.2p", "pres.3p"]
    );
    assert_eq!(french[6], "pc.1s");
    assert_eq!(french[12], "impf.1s");
    assert_eq!(french[18], "fut.1s");
    assert_eq!(french[24], "cond.1s");
    assert_eq!(french[30], "subj.1s");
    assert_eq!(&french[36..], ["imp.2s", "imp.1p", "imp.2p"]);
    assert_eq!(label_at("fr", 0), "present");
    assert_eq!(label_at("fr", 6), "present perfect");
    assert_eq!(label_at("fr", 12), "imperfect");
    assert_eq!(label_at("fr", 18), "future");
    assert_eq!(label_at("fr", 24), "conditional");
    assert_eq!(label_at("fr", 30), "subjunctive");
    assert_eq!(label_at("fr", 36), "imperative");

    let italian = ids("it");
    assert_eq!(italian[6], "pp.1s");
    assert_eq!(italian[30], "cong.1s");
    assert!(!italian.contains(&"pc.1s") && !italian.contains(&"subj.1s"));
    assert_eq!(label_at("it", 6), "present perfect");
    assert_eq!(label_at("it", 30), "subjunctive");

    let spanish = ids("es");
    assert_eq!(spanish[6], "pret.1s");
    assert_eq!(spanish[30], "subj.1s");
    assert!(!spanish.contains(&"pc.1s") && !spanish.contains(&"cong.1s"));
    assert_eq!(label_at("es", 6), "preterite");

    assert_eq!(label_at("gd", 0), "past");
    assert_eq!(label_at("gd", 1), "future");
    assert_eq!(label_at("gd", 2), "conditional");
    assert_eq!(label_at("gd", 4), "imperative");
    assert_eq!(label_at("gd", 6), "verbal noun");
}

#[test]
fn slot_table_refuses_a_language_it_does_not_list() {
    let missing = slots_for("en").expect_err("english has no paradigm");
    assert!(format!("{missing}").contains("no paradigm slots for en"));
    let blank = slots_for("").expect_err("empty");
    assert!(format!("{blank}").contains("Invalid language code"));
    let upper = slots_for("FR").expect_err("case sensitive");
    assert!(format!("{upper}").contains("no paradigm slots for FR"));
}

#[test]
fn validate_paradigm_accepts_a_compound_form_and_returns_table_order() {
    let mut answer = filled("fr");
    answer
        .iter_mut()
        .find(|slot| slot.slot == "pc.1p")
        .expect("pc.1p")
        .form = "avons   parlé".to_owned();
    answer.reverse();
    let validated = validate_paradigm("fr", answer).expect("french");
    assert_eq!(validated.len(), 39);
    assert_eq!(
        validated.iter().map(|slot| slot.slot).collect::<Vec<_>>(),
        ids("fr")
    );
    let compound = validated
        .iter()
        .find(|slot| slot.slot == "pc.1p")
        .expect("pc.1p");
    assert_eq!(compound.form, "avons parlé");
    assert_eq!(compound.label, "present perfect");

    let mut gaelic = filled("gd");
    gaelic
        .iter_mut()
        .find(|slot| slot.slot == "vn")
        .expect("vn")
        .form = "a' dol".to_owned();
    let validated = validate_paradigm("gd", gaelic).expect("gaelic");
    assert_eq!(
        validated
            .iter()
            .find(|slot| slot.slot == "vn")
            .expect("vn")
            .form,
        "a' dol"
    );
}

#[test]
fn validate_paradigm_rejects_the_wrong_shape() {
    let mut missing = filled("fr");
    missing.pop();
    assert!(validate_paradigm("fr", missing).is_err());

    let mut extra = filled("fr");
    extra.push(ParadigmSlot {
        slot: "imp.1s".to_owned(),
        form: "parle".to_owned(),
        native: "I speak".to_owned(),
    });
    assert!(validate_paradigm("fr", extra).is_err());

    let mut duplicated = filled("es");
    duplicated[1].slot = duplicated[0].slot.clone();
    assert!(validate_paradigm("es", duplicated).is_err());

    let mut blank_form = filled("it");
    blank_form[0].form = "   ".to_owned();
    assert!(validate_paradigm("it", blank_form).is_err());

    let mut too_many = filled("fr");
    too_many[0].form = words(FORM_WORDS_MAX.saturating_add(1));
    let rejected = validate_paradigm("fr", too_many).expect_err("four words");
    assert!(format!("{rejected}").contains("pres.1s"));

    let mut three = filled("fr");
    three[0].form = words(FORM_WORDS_MAX);
    assert!(validate_paradigm("fr", three).is_ok());

    let mut long_word = filled("gd");
    long_word[0].form = "a".repeat(MAX_WORD_CHARS.saturating_add(1));
    assert!(validate_paradigm("gd", long_word).is_err());
    let mut boundary = filled("gd");
    boundary[0].form = "a".repeat(MAX_WORD_CHARS);
    assert!(validate_paradigm("gd", boundary).is_ok());

    let mut blank_native = filled("es");
    blank_native[0].native = "  ".to_owned();
    assert!(validate_paradigm("es", blank_native).is_err());

    let mut long_native = filled("es");
    long_native[0].native = words(NATIVE_WORDS_MAX.saturating_add(1));
    assert!(validate_paradigm("es", long_native).is_err());
    let mut native_boundary = filled("es");
    native_boundary[0].native = words(NATIVE_WORDS_MAX);
    assert!(validate_paradigm("es", native_boundary).is_ok());

    let mut wide_form = filled("fr");
    wide_form[0].form = format!("{} {} {}", "a".repeat(30), "b".repeat(30), "c".repeat(30));
    let rejected = validate_paradigm("fr", wide_form).expect_err("form column");
    assert!(format!("{rejected}").contains(&FORM_CHARS_MAX.to_string()));

    let mut wide_face = filled("es");
    wide_face[0].native = std::iter::repeat_n("n".repeat(24), NATIVE_WORDS_MAX)
        .collect::<Vec<_>>()
        .join(" ");
    let rejected = validate_paradigm("es", wide_face).expect_err("face column");
    assert!(format!("{rejected}").contains(&NATIVE_FACE_CHARS_MAX.to_string()));
}

#[test]
fn native_face_uses_the_middle_dot() {
    let face = native_face("we go", "present");
    assert_eq!(face, "we go · present");
    assert!(face.contains('\u{00b7}'));
}

#[test]
fn paradigm_prompt_names_every_slot_and_is_stable() {
    let slots = slots_for("fr").expect("fr");
    let prompt_slots: Vec<ParadigmPromptSlot<'_>> = slots
        .iter()
        .map(|slot| ParadigmPromptSlot {
            id: slot.id,
            label: slot.label,
        })
        .collect();
    let lemma = "say \"go\"";
    let first = paradigm_prompt("French", "English", lemma, &prompt_slots).expect("prompt");
    let second = paradigm_prompt("French", "English", lemma, &prompt_slots).expect("prompt");
    assert_eq!(first, second);
    assert!(first.contains("say \\\"go\\\""));
    assert!(first.contains("French") && first.contains("English"));
    assert!(first.contains(&format!(
        "Each word of form is at most {MAX_WORD_CHARS} characters"
    )));
    assert!(first.contains(&format!(
        "The whole form is at most {FORM_CHARS_MAX} characters"
    )));
    assert!(first.contains(&format!("at most {NATIVE_FACE_CHARS_MAX} characters")));
    assert!(first.contains(&format!("one to {NATIVE_WORDS_MAX} words")));
    assert!(first.contains(&format!("one to {FORM_WORDS_MAX} words")));
    for slot in slots {
        assert!(first.contains(slot.id), "{}", slot.id);
    }
    let padded = paradigm_prompt("French", "English", "  parler  ", &prompt_slots).expect("trim");
    let bare = paradigm_prompt("French", "English", "parler", &prompt_slots).expect("bare");
    assert_eq!(padded, bare);
    assert!(paradigm_prompt("French", "English", "a b", &prompt_slots).is_err());
    assert!(paradigm_prompt("French", "English", "", &prompt_slots).is_err());
    assert!(paradigm_prompt("French", "English", "parler", &[]).is_err());
}
