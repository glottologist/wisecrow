use dioxus::prelude::*;

use crate::router::Route;

#[component]
pub fn Layout() -> Element {
    rsx! {
        div {
            style: "min-height: 100vh; display: flex; flex-direction: column; background: #111827; color: #f3f4f6; font-family: -apple-system, BlinkMacSystemFont, sans-serif;",

            div {
                style: "flex: 1; overflow-y: auto; padding: 16px; padding-bottom: 80px;",
                Outlet::<Route> {}
            }

            nav {
                style: "position: fixed; bottom: 0; left: 0; right: 0; display: flex; justify-content: space-around; background: #1f2937; padding: 12px 0; border-top: 1px solid #374151;",
                Link {
                    to: Route::Home {},
                    style: "text-align: center; color: #9ca3af; font-size: 12px; text-decoration: none; padding: 8px 16px; min-width: 64px;",
                    "Home"
                }
            }
        }
    }
}
