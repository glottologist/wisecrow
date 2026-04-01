use dioxus::prelude::*;
use wisecrow_dto::RuleContextDto;

#[component]
pub fn RuleExplanation(context: RuleContextDto) -> Element {
    let mut expanded = use_signal(|| false);

    rsx! {
        div { class: "bg-gray-700 rounded-lg p-4 mt-4",
            button {
                class: "flex items-center justify-between w-full text-left",
                onclick: move |_| expanded.set(!expanded()),
                div {
                    span { class: "text-sm text-cyan-400 font-semibold", "{context.cefr_level}" }
                    span { class: "text-white ml-2", "{context.rule_title}" }
                }
                span { class: "text-gray-400", if expanded() { "^" } else { "v" } }
            }

            if expanded() {
                div { class: "mt-3 space-y-3",
                    p { class: "text-gray-300 text-sm", "{context.rule_explanation}" }

                    if !context.extra_examples.is_empty() {
                        div { class: "mt-2",
                            p { class: "text-xs text-gray-500 uppercase tracking-wide", "Examples" }
                            ul { class: "list-disc list-inside text-sm text-gray-400 mt-1",
                                for example in &context.extra_examples {
                                    li { "{example}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
