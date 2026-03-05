use dioxus::prelude::*;

#[component]
pub fn TimerBar(fraction: f64) -> Element {
    let pct = (fraction * 100.0).clamp(0.0, 100.0);
    let width = format!("{pct:.0}%");

    let color = if pct > 50.0 {
        "bg-emerald-500"
    } else if pct > 20.0 {
        "bg-yellow-500"
    } else {
        "bg-red-500"
    };

    rsx! {
        div { class: "w-full bg-gray-700 rounded-full h-2",
            div {
                class: "h-2 rounded-full transition-all duration-100 {color}",
                style: "width: {width}",
            }
        }
    }
}
