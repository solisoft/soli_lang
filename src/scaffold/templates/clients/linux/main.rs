//! {{APP_NAME}} Linux shell — WebKitGTK + Soli native bridge.

use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Orientation};
use std::env;
use webkit2gtk::prelude::*;
use webkit2gtk::{
    SettingsExt, UserContentManager, UserContentManagerExt, UserScript,
    UserScriptInjectionTime, WebView, WebViewExt,
};

const START_URL: &str = "{{START_URL}}";
const SCHEME: &str = "{{SCHEME}}";

fn main() {
    let app = Application::builder()
        .application_id("{{PACKAGE_ID}}")
        .build();

    app.connect_activate(|app| {
        let open_url = env::args().nth(1).and_then(|a| map_deep_link(&a));
        build_ui(app, open_url.as_deref().unwrap_or(START_URL));
    });

    app.connect_open(move |app, files, _| {
        if let Some(f) = files.first() {
            if let Some(uri) = f.uri() {
                if let Some(mapped) = map_deep_link(&uri) {
                    build_ui(app, &mapped);
                    return;
                }
            }
        }
        build_ui(app, START_URL);
    });

    app.run();
}

fn map_deep_link(arg: &str) -> Option<String> {
    let arg = arg.trim();
    let prefix = format!("{SCHEME}://");
    if arg.starts_with(&prefix) {
        let rest = arg.trim_start_matches(&format!("{SCHEME}:"));
        let rest = rest.trim_start_matches("//");
        let path = if let Some(idx) = rest.find('/') {
            &rest[idx..]
        } else {
            "/"
        };
        let base = START_URL.trim_end_matches('/');
        return Some(format!("{base}{path}"));
    }
    if arg.starts_with("https://") || arg.starts_with("http://") {
        return Some(arg.to_string());
    }
    if arg.starts_with('/') {
        let base = START_URL.trim_end_matches('/');
        return Some(format!("{base}{arg}"));
    }
    None
}

fn build_ui(app: &Application, url: &str) {
    for w in app.windows() {
        if let Ok(win) = w.downcast::<ApplicationWindow>() {
            if let Some(child) = win.child() {
                if let Ok(box_) = child.downcast::<gtk::Box>() {
                    if let Some(wv) = box_.first_child().and_then(|c| c.downcast::<WebView>().ok()) {
                        wv.load_uri(url);
                        win.present();
                        return;
                    }
                }
            }
        }
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("{{APP_NAME}}")
        .default_width(420)
        .default_height(780)
        .build();

    let ucm = UserContentManager::new();
    let bridge = r#"
        window.soli = window.soli || {};
        window.soli.native = {
          platform: "linux",
          capabilities: ["notify","geolocation","share","print","clipboard","badge"],
          notify: function(json) {
            window.webkit.messageHandlers.soliNative.postMessage(
              typeof json === "string" ? json : JSON.stringify(json)
            );
          },
          call: function(json) {
            window.webkit.messageHandlers.soliNative.postMessage(
              typeof json === "string" ? json : JSON.stringify(json)
            );
          }
        };
    "#;
    let script = UserScript::new(bridge, UserScriptInjectionTime::Start, &[], &[]);
    ucm.add_script(&script);

    let webview = WebView::new_with_context_and_user_content_manager(
        &webkit2gtk::WebContext::default().unwrap(),
        &ucm,
    );
    if let Some(settings) = WebViewExt::settings(&webview) {
        settings.set_enable_developer_extras(true);
        settings.set_javascript_can_access_clipboard(true);
    }

    let box_ = gtk::Box::new(Orientation::Vertical, 0);
    box_.append(&webview);
    window.set_child(Some(&box_));
    webview.load_uri(url);
    window.present();
}
