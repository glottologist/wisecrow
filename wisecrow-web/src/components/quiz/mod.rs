mod cloze;
mod multiple_choice;

use dioxus::prelude::*;

use wisecrow_dto::QuizItemDto;

#[cfg(feature = "server")]
use crate::server::quiz::generate_quiz;

#[cfg(not(feature = "server"))]
mod server_stubs {
    use dioxus::prelude::*;
    use wisecrow_dto::QuizItemDto;

    #[server]
    pub async fn generate_quiz(
        pdf_bytes: Vec<u8>,
        num_questions: u32,
    ) -> Result<Vec<QuizItemDto>, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }
}

#[cfg(not(feature = "server"))]
use server_stubs::*;

#[component]
pub fn QuizPage() -> Element {
    let mut items: Signal<Vec<QuizItemDto>> = use_signal(Vec::new);
    let mut current_index = use_signal(|| 0usize);
    let mut correct_count = use_signal(|| 0usize);
    let mut total_answered = use_signal(|| 0usize);
    let mut loading = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut started = use_signal(|| false);

    if !started() {
        return rsx! {
            div { class: "max-w-lg mx-auto space-y-6",
                h1 { class: "text-3xl font-bold text-center", "Grammar Quiz" }
                p { class: "text-gray-400 text-center",
                    "Upload a PDF grammar guide to generate quiz questions."
                }

                if loading() {
                    div { class: "text-center text-gray-400 py-8", "Generating quiz..." }
                } else {
                    form {
                        class: "bg-gray-800 rounded-xl p-6 space-y-4",
                        input {
                            r#type: "file",
                            accept: ".pdf",
                            class: "w-full text-gray-300",
                            onchange: move |evt: Event<FormData>| {
                                async move {
                                    let files = evt.data.files();
                                    if let Some(file) = files.first() {
                                        loading.set(true);
                                        error_msg.set(None);
                                        match file.read_bytes().await {
                                            Ok(bytes) => {
                                                match generate_quiz(bytes.to_vec(), 20).await {
                                                    Ok(quiz_items) => {
                                                        if quiz_items.is_empty() {
                                                            error_msg.set(Some("No quiz questions could be generated from this PDF.".to_owned()));
                                                        } else {
                                                            items.set(quiz_items);
                                                            started.set(true);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        error_msg.set(Some(format!("Quiz generation failed: {e}")));
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                error_msg.set(Some(format!("Failed to read file: {e}")));
                                            }
                                        }
                                        loading.set(false);
                                    }
                                }
                            },
                        }
                    }
                }

                if let Some(err) = error_msg() {
                    div { class: "text-red-400 text-center", "{err}" }
                }
            }
        };
    }

    let all_items = items();
    let idx = current_index();
    let total = all_items.len();

    if idx >= total {
        let pct = if total_answered() > 0 {
            correct_count()
                .saturating_mul(100)
                .checked_div(total_answered())
                .unwrap_or(0)
        } else {
            0
        };

        return rsx! {
            div { class: "text-center space-y-4 py-20",
                h2 { class: "text-3xl font-bold text-emerald-400", "Quiz Complete!" }
                p { class: "text-xl text-gray-300",
                    "{correct_count()}/{total_answered()} correct ({pct}%)"
                }
                button {
                    class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-3 font-semibold transition",
                    onclick: move |_| {
                        started.set(false);
                        items.set(Vec::new());
                        current_index.set(0);
                        correct_count.set(0);
                        total_answered.set(0);
                    },
                    "Try Another PDF"
                }
            }
        };
    }

    let display_num = idx.saturating_add(1);

    rsx! {
        div { class: "max-w-2xl mx-auto space-y-4",
            div { class: "flex justify-between text-sm text-gray-500",
                span { "Question {display_num} / {total}" }
                span { "Score: {correct_count()}/{total_answered()}" }
            }

            match &all_items[idx] {
                QuizItemDto::Cloze(q) => rsx! {
                    cloze::ClozeQuestion {
                        quiz: q.clone(),
                        on_answer: move |correct: bool| {
                            total_answered.set(total_answered().saturating_add(1));
                            if correct {
                                correct_count.set(correct_count().saturating_add(1));
                            }
                        },
                        on_next: move |_| {
                            current_index.set(idx.saturating_add(1));
                        },
                    }
                },
                QuizItemDto::MultipleChoice(q) => rsx! {
                    multiple_choice::McQuestion {
                        quiz: q.clone(),
                        on_answer: move |correct: bool| {
                            total_answered.set(total_answered().saturating_add(1));
                            if correct {
                                correct_count.set(correct_count().saturating_add(1));
                            }
                        },
                        on_next: move |_| {
                            current_index.set(idx.saturating_add(1));
                        },
                    }
                },
            }
        }
    }
}
