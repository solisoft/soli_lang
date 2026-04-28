use std::collections::HashMap;
use std::path::Path;

use crate::interpreter::Interpreter;

use super::RequestData;

/// Helper function to render error page with full details.
pub(super) fn render_error_page(
    error_msg: &str,
    interpreter: &Interpreter,
    request_data: &RequestData,
    stack_trace: &[String],
    breakpoint_env_json: Option<&str>,
) -> String {
    let error_type = if breakpoint_env_json.is_some() {
        "Breakpoint"
    } else {
        "RuntimeError"
    };

    let captured_env = if let Some(env) = breakpoint_env_json {
        env.to_string()
    } else {
        interpreter.serialize_environment_for_debug()
    };
    let env_json_for_render: Option<&str> = Some(&captured_env);

    let mut full_stack_trace: Vec<String> = Vec::new();
    let (actual_error, embedded_stack): (String, Vec<String>) =
        if let Some(stack_start) = error_msg.find("Stack trace:\n") {
            let error_part = error_msg[..stack_start].trim().to_string();
            let stack_part = error_msg[stack_start + "Stack trace:\n".len()..]
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            (error_part, stack_part)
        } else {
            (error_msg.to_string(), Vec::new())
        };

    let span_info = extract_span_from_error(&actual_error);
    let error_line = span_info.line;
    // Prefer the deepest stack frame's file: when an error happens inside
    // a callee, the error span's line belongs to that callee, not to the
    // outermost (controller) frame. Picking the first frame would yield a
    // file/line mismatch like `controller.sl:92` when line 92 is in the
    // deeper `service.sl`.
    let error_file = span_info
        .file
        .clone()
        .or_else(|| {
            for frame in embedded_stack.iter().rev() {
                if let Some(file) = extract_file_from_frame(frame) {
                    return Some(file);
                }
            }
            for frame in stack_trace.iter().rev() {
                if let Some(file) = extract_file_from_frame(frame) {
                    return Some(file);
                }
            }
            None
        })
        .unwrap_or_else(|| "unknown".to_string());

    let location = format!("{}:{}", error_file, error_line);
    full_stack_trace.push(format!("Error: {}", actual_error));
    full_stack_trace.extend(embedded_stack);
    full_stack_trace.extend(stack_trace.iter().cloned());

    static VIEW_PAT1: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"at (\d+):(\d+) in ([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb))",
        ).unwrap()
    });
    static VIEW_PAT2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"in ([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb)) at (\d+):(\d+)",
        ).unwrap()
    });
    static VIEW_PAT3: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"at ([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb)):(\d+)",
        )
        .unwrap()
    });
    static VIEW_PAT4: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"in ([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb))(?:\s|$)",
        ).unwrap()
    });

    let mut view_added = false;
    if let Some(caps) = VIEW_PAT1.captures(&actual_error) {
        let view_line = caps.get(1).map(|m| m.as_str()).unwrap_or("1");
        let view_file = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        if !view_file.is_empty() {
            full_stack_trace.push(format!("[view] at {}:{}", view_file, view_line));
            view_added = true;
        }
    }
    if !view_added {
        if let Some(caps) = VIEW_PAT2.captures(&actual_error) {
            let view_file = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let view_line = caps.get(2).map(|m| m.as_str()).unwrap_or("1");
            if !view_file.is_empty() {
                full_stack_trace.push(format!("[view] at {}:{}", view_file, view_line));
                view_added = true;
            }
        }
    }
    if !view_added {
        if let Some(caps) = VIEW_PAT3.captures(&actual_error) {
            let view_file = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let view_line = caps.get(2).map(|m| m.as_str()).unwrap_or("1");
            if !view_file.is_empty() {
                full_stack_trace.push(format!("[view] at {}:{}", view_file, view_line));
                view_added = true;
            }
        }
    }
    if !view_added {
        if let Some(caps) = VIEW_PAT4.captures(&actual_error) {
            let view_file = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if !view_file.is_empty() {
                full_stack_trace.push(format!("[view] at {}:1", view_file));
            }
        }
    }

    if full_stack_trace.len() == 1 {
        full_stack_trace.push(format!("{}:{} (error location)", error_file, error_line));
    }

    let mut source_files: HashMap<String, String> = HashMap::new();
    let app_root = crate::live::component::get_app_root();
    static SOURCE_FILE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb|\.sl)):(\d+)",
        ).unwrap()
    });
    for frame in &full_stack_trace {
        if let Some(caps) = SOURCE_FILE_RE.captures(frame) {
            if let Some(file_match) = caps.get(1) {
                let file_str = file_match.as_str();
                if !source_files.contains_key(file_str) {
                    let candidates = [
                        std::path::Path::new(file_str).to_path_buf(),
                        app_root.join(file_str),
                    ];
                    for candidate in &candidates {
                        if candidate.exists() {
                            if let Ok(content) = std::fs::read_to_string(candidate) {
                                let lines_map: HashMap<usize, String> = content
                                    .lines()
                                    .enumerate()
                                    .map(|(i, l)| (i + 1, l.to_string()))
                                    .collect();
                                if let Ok(json) = serde_json::to_string(&lines_map) {
                                    source_files.insert(file_str.to_string(), json);
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    let query_json = format!("{:?}", request_data.query);
    let headers_json = format!("{:?}", request_data.headers);
    let request_data_json = format!(
        r#"{{"method":"{}","path":"{}","params":{},"query":{},"headers":{},"body":"{}","session":"N/A"}}"#,
        request_data.method,
        request_data.path,
        query_json,
        query_json,
        headers_json,
        request_data.body
    );

    render_dev_error_page(
        &actual_error,
        error_type,
        &location,
        &full_stack_trace,
        &request_data_json,
        env_json_for_render,
        &source_files,
    )
}

fn extract_json_field(json: &str, field: &str) -> Option<String> {
    let pattern = format!(r#""{}":"#, field);
    if let Some(start) = json.find(&pattern) {
        let after_start = start + pattern.len();
        let mut end = after_start;
        let mut depth = 0;
        let chars: Vec<char> = json[after_start..].chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if *c == '{' || *c == '[' {
                depth += 1;
            } else if *c == '}' || *c == ']' {
                depth -= 1;
                if depth == 0 {
                    end = after_start + i + 1;
                    break;
                }
            } else if *c == ',' && depth == 0 {
                end = after_start + i;
                break;
            }
        }
        return Some(json[after_start..end].to_string());
    }
    None
}

#[allow(dead_code)]
struct SpanInfo {
    file: Option<String>,
    line: usize,
    column: usize,
}

fn extract_span_from_error(error_msg: &str) -> SpanInfo {
    static FILE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb|\.sl))",
        )
        .unwrap()
    });
    static AT_FILE_LINE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"at ([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb)):(\d+)",
        )
        .unwrap()
    });
    static SPAN_RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r" at (\d+):(\d+)").unwrap());
    static FILE_LINE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb|\.sl)):(\d+)",
        ).unwrap()
    });
    static LINE_RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?:at\s+)?line\s*[=:]\s*(\d+)").unwrap());

    let file = FILE_RE
        .captures(error_msg)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string());

    if let Some(caps) = AT_FILE_LINE_RE.captures(error_msg) {
        let file = caps.get(1).map(|m| m.as_str().to_string());
        let line = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(1);
        return SpanInfo {
            file,
            line,
            column: 1,
        };
    }
    if let Some(caps) = SPAN_RE.captures(error_msg) {
        let line = caps
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(1);
        let column = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(1);
        return SpanInfo { file, line, column };
    }
    if let Some(caps) = FILE_LINE_RE.captures(error_msg) {
        let file = caps.get(1).map(|m| m.as_str().to_string());
        let line = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(1);
        return SpanInfo {
            file,
            line,
            column: 1,
        };
    }
    if let Some(caps) = LINE_RE.captures(error_msg) {
        let line = caps
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(1);
        return SpanInfo {
            file,
            line,
            column: 1,
        };
    }

    SpanInfo {
        file,
        line: 1,
        column: 1,
    }
}

fn extract_file_from_frame(frame: &str) -> Option<String> {
    static FILE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb|\.sl))",
        )
        .unwrap()
    });
    FILE_RE
        .captures(frame)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

pub(super) fn render_dev_error_page(
    error: &str,
    error_type: &str,
    location: &str,
    stack_trace: &[String],
    request_data_json: &str,
    breakpoint_env_json: Option<&str>,
    preloaded_sources: &HashMap<String, String>,
) -> String {
    let error_message = escape_html(error);
    let error_type = escape_html(error_type);
    let error_location = escape_html(location);
    let mut stack_frames = Vec::new();
    let mut frame_index = 0;

    static FILE_REGEX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb|\.sl)):(\d+)",
        ).unwrap()
    });
    static SPAN_REGEX: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r" at (\d+):(\d+)").unwrap());
    static VIEW_FILE_REGEX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"in ([./a-zA-Z0-9_@-]+(?:\.html\.slv|\.slv|\.html\.md|\.md|\.html\.erb|\.erb))",
        )
        .unwrap()
    });

    for frame in stack_trace {
        if frame.starts_with("Error: ") {
            continue;
        }
        let is_view_frame = frame.starts_with("[view]");
        let mut file = "unknown".to_string();
        let mut line: usize = 0;

        if let Some(caps) = FILE_REGEX.captures(frame) {
            file = caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            line = caps
                .get(2)
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
        } else if let Some(caps) = VIEW_FILE_REGEX.captures(frame) {
            file = caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }
        if let Some(caps) = SPAN_REGEX.captures(frame) {
            if let Some(span_line) = caps.get(1).and_then(|m| m.as_str().parse().ok()) {
                line = span_line;
            }
        }

        let contains_source_ext = |s: &str| {
            s.contains(".sl")
                || s.contains(".html.slv")
                || s.contains(".slv")
                || s.contains(".html.md")
                || s.contains(".md")
                || s.contains(".html.erb")
                || s.contains(".erb")
        };

        let (display_name, icon_html) = if is_view_frame {
            let view_name = file.rsplit('/').next().unwrap_or(&file);
            (
                view_name.to_string(),
                r#"<svg class="inline-block w-4 h-4 mr-1.5 -mt-0.5 text-teal-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path></svg>"#,
            )
        } else {
            let func = if let Some(at_pos) = frame.find(" at ") {
                let before_at = &frame[..at_pos];
                if !contains_source_ext(before_at) {
                    before_at.to_string()
                } else {
                    extract_controller_name(&file)
                }
            } else if file != "unknown" {
                extract_controller_name(&file)
            } else {
                frame.clone()
            };

            let display = if func.contains('#') || func.contains("::") {
                func.clone()
            } else if func.contains('/') || contains_source_ext(&func) {
                extract_controller_name(&func)
            } else if func == "unknown" && file != "unknown" {
                extract_controller_name(&file)
            } else {
                func.clone()
            };
            (display, "")
        };

        let location_display = format!("{}:{}", file, line);
        let (name_color, location_color, border_color) = if is_view_frame {
            (
                "text-teal-300",
                "text-teal-400/70",
                "border-l-2 border-teal-400",
            )
        } else {
            ("text-white", "text-gray-400", "")
        };

        stack_frames.push(format!(
            r#"<div class="stack-frame px-4 py-3 border-b border-white/5 hover:bg-white/5 transition-colors {}" onclick="showSource('{}', {}, this)">
                <div class="flex items-start gap-3">
                    <span class="text-gray-500 text-xs mt-0.5">{}</span>
                    <div class="flex-1 min-w-0">
                        <div class="font-medium {} truncate">{}{}</div>
                        <div class="{} text-sm truncate">{}</div>
                    </div>
                </div>
            </div>"#,
            border_color,
            escape_html(&file),
            line,
            frame_index,
            name_color,
            icon_html,
            escape_html(&display_name),
            location_color,
            escape_html(&location_display)
        ));
        frame_index += 1;
    }

    let preloaded_sources_js = if preloaded_sources.is_empty() {
        "{}".to_string()
    } else {
        let entries: Vec<String> = preloaded_sources
            .iter()
            .map(|(file, lines_json)| {
                format!(
                    r#""{}": {{"lines": {}, "line": 1}}"#,
                    file.replace('\\', "\\\\").replace('"', "\\\""),
                    lines_json
                )
            })
            .collect();
        format!("{{{}}}", entries.join(","))
    };

    let request_method =
        extract_json_field(request_data_json, "method").unwrap_or("UNKNOWN".to_string());
    let request_path = extract_json_field(request_data_json, "path").unwrap_or("/".to_string());
    let request_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Error - {error_type}</title>
    <script src="https://cdn.tailwindcss.com"></script>
    <style>
        .code-editor {{ font-family: 'JetBrains Mono', 'Fira Code', monospace; font-size: 14px; line-height: 1.6; }}
        .repl-output {{ min-height: 100px; max-height: 400px; overflow-y: auto; }}
        .stack-frame {{ cursor: pointer; }}
        .stack-frame:hover {{ background-color: rgba(99, 102, 241, 0.1); }}
        .stack-frame.active {{ background-color: rgba(99, 102, 241, 0.2); border-left: 3px solid #6366f1; }}
        .section-content {{ display: none; }}
        .section-content.active {{ display: block; }}
        .request-tab.active {{ background-color: rgba(99, 102, 241, 0.2); border-bottom: 2px solid #6366f1; }}
        .loading-spinner {{ border: 2px solid rgba(255,255,255,0.3); border-top: 2px solid #6366f1; border-radius: 50%; width: 16px; height: 16px; animation: spin 1s linear infinite; }}
        @keyframes spin {{ 0% {{ transform: rotate(0deg); }} 100% {{ transform: rotate(360deg); }} }}
    </style>
</head>
<body class="bg-gray-950 text-gray-100 min-h-screen">
    <div class="max-w-7xl mx-auto p-6">
        <div class="mb-8 border-b border-white/10 pb-6">
            <div class="flex items-center gap-3 mb-2">
                <div class="px-3 py-1 rounded-full bg-red-500/20 text-red-400 text-sm font-medium">{error_type}</div>
                <span class="text-gray-500">Development Mode</span>
            </div>
            <h1 class="text-3xl font-bold text-white mb-2">{error_message}</h1>
            <p class="text-gray-400">{error_location}</p>
        </div>
        <div class="mb-8 rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
            <div class="flex items-center justify-between px-4 py-3 bg-gray-800 border-b border-white/10">
                <div class="flex items-center gap-2">
                    <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                    </svg>
                    <span class="font-semibold text-white">Interactive REPL</span>
                </div>
                <button onclick="clearRepl()" class="text-gray-400 hover:text-white text-sm">Clear</button>
            </div>
            <div class="p-4">
                <div class="flex gap-2 mb-3">
                    <input type="text" id="repl-input" class="flex-1 bg-gray-800 border border-white/20 rounded-lg px-4 py-2 text-white placeholder-gray-500 focus:outline-none focus:border-indigo-500 code-editor" placeholder="Type Soli code to inspect request state..." onkeydown="if(event.key==='Enter'&&!event.shiftKey){{event.preventDefault();executeRepl();}}">
                    <button onclick="executeRepl()" class="px-6 py-2 bg-indigo-600 hover:bg-indigo-700 text-white rounded-lg font-medium transition-colors flex items-center gap-2">
                        <span>Run</span>
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z" />
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                        </svg>
                    </button>
                </div>
                <div id="repl-output" class="repl-output bg-gray-800 rounded-lg p-4 text-sm code-editor min-h-[120px]">
                    <div class="text-gray-500 italic">// Try: req["params"]["id"] or session["user_id"] or headers["Content-Type"]</div>
                </div>
            </div>
        </div>
        <div class="grid grid-cols-1 lg:grid-cols-3 gap-8">
            <div class="lg:col-span-2">
                <div class="mb-6">
                    <h2 class="text-xl font-bold text-white mb-4 flex items-center gap-2">
                        <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
                        </svg>
                        Stack Trace
                    </h2>
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        {stack_frames}
                    </div>
                </div>
                <div class="mb-6">
                    <h2 class="text-xl font-bold text-white mb-4 flex items-center gap-2">
                        <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                        </svg>
                        Source Code
                    </h2>
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <div class="px-4 py-2 bg-gray-800 border-b border-white/10 flex items-center justify-between">
                            <span id="source-file" class="text-sm text-gray-400 font-mono">Select a stack frame to view source</span>
                            <span id="source-line" class="text-sm text-gray-500"></span>
                        </div>
                        <pre id="source-code" class="p-4 overflow-x-auto code-editor text-sm"><code class="language-soli text-gray-400">// Click on a stack frame above to see the source code</code></pre>
                    </div>
                </div>
                <div class="mb-6">
                    <h2 class="text-xl font-bold text-white mb-4 flex items-center gap-2">
                        <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                        </svg>
                        Quick Inspect
                    </h2>
                    <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
                        <button onclick="quickInspect('req')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors"><div class="text-xs text-gray-500 mb-1">Request</div><div class="text-sm text-white font-mono truncate">req</div></button>
                        <button onclick="quickInspect('req[\"params\"]')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors"><div class="text-xs text-gray-500 mb-1">Params</div><div class="text-sm text-white font-mono truncate">params</div></button>
                        <button onclick="quickInspect('req[\"query\"]')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors"><div class="text-xs text-gray-500 mb-1">Query</div><div class="text-sm text-white font-mono truncate">query</div></button>
                        <button onclick="quickInspect('req[\"body\"]')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors"><div class="text-xs text-gray-500 mb-1">Body</div><div class="text-sm text-white font-mono truncate">body</div></button>
                        <button onclick="quickInspect('session')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors"><div class="text-xs text-gray-500 mb-1">Session</div><div class="text-sm text-white font-mono truncate">session</div></button>
                        <button onclick="quickInspect('headers')" class="p-3 rounded-lg bg-gray-800 hover:bg-gray-700 border border-white/10 text-left transition-colors"><div class="text-xs text-gray-500 mb-1">Headers</div><div class="text-sm text-white font-mono truncate">headers</div></button>
                    </div>
                </div>
            </div>
            <div>
                <h2 class="text-xl font-bold text-white mb-4 flex items-center gap-2">
                    <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
                    </svg>
                    Request Details
                </h2>
                <div class="flex border-b border-white/10 mb-4">
                    <button class="request-tab active px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('params')">Params</button>
                    <button class="request-tab px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('query')">Query</button>
                    <button class="request-tab px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('body')">Body</button>
                    <button class="request-tab px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('headers')">Headers</button>
                    <button class="request-tab px-4 py-2 text-sm text-gray-400 hover:text-white transition-colors" onclick="showRequestTab('session')">Session</button>
                </div>
                <div id="tab-params" class="section-content active"><div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden"><pre id="json-params" class="p-4 overflow-x-auto text-sm code-editor"></pre></div></div>
                <div id="tab-query" class="section-content"><div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden"><pre id="json-query" class="p-4 overflow-x-auto text-sm code-editor"></pre></div></div>
                <div id="tab-body" class="section-content"><div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden"><pre id="json-body" class="p-4 overflow-x-auto text-sm code-editor"></pre></div></div>
                <div id="tab-headers" class="section-content"><div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden"><pre id="json-headers" class="p-4 overflow-x-auto text-sm code-editor"></pre></div></div>
                <div id="tab-session" class="section-content"><div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden"><pre id="json-session" class="p-4 overflow-x-auto text-sm code-editor"></pre></div></div>
                <div class="mt-6">
                    <h3 class="text-lg font-semibold text-white mb-3">Environment</h3>
                    <div class="rounded-xl bg-gray-900 border border-white/10 overflow-hidden">
                        <div class="divide-y divide-white/5">
                            <div class="px-4 py-2 flex justify-between"><span class="text-gray-500">Time</span><span class="text-gray-300 font-mono text-sm">{request_time}</span></div>
                            <div class="px-4 py-2 flex justify-between"><span class="text-gray-500">Method</span><span class="text-gray-300 font-mono text-sm">{request_method}</span></div>
                            <div class="px-4 py-2 flex justify-between"><span class="text-gray-500">Path</span><span class="text-gray-300 font-mono text-sm truncate ml-2">{request_path}</span></div>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    </div>
    <script>
        const sourceCache = {{}};
        const preloadedFiles = {preloaded_sources_js};
        const currentRequestData = {request_data_json};
        const breakpointEnv = {breakpoint_env_js};
        function showRequestTab(tabName) {{
            document.querySelectorAll('.section-content').forEach(el => el.classList.remove('active'));
            document.querySelectorAll('.request-tab').forEach(el => el.classList.remove('active'));
            document.getElementById('tab-' + tabName).classList.add('active');
            event.target.classList.add('active');
        }}
        async function showSource(file, line, element) {{
            document.querySelectorAll('.stack-frame').forEach(el => el.classList.remove('active'));
            element.classList.add('active');
            document.getElementById('source-file').textContent = file + ':' + line;
            document.getElementById('source-line').textContent = 'Line ' + line;
            const cacheKey = file + ':' + line;
            if (sourceCache[cacheKey]) {{ displaySource(sourceCache[cacheKey], line); return; }}
            if (preloadedFiles[file]) {{
                const data = {{ file: file, line: line, lines: preloadedFiles[file].lines }};
                sourceCache[cacheKey] = data;
                displaySource(data, line);
                return;
            }}
            try {{
                const response = await fetch('/__dev/source?file=' + encodeURIComponent(file) + '&line=' + line);
                if (response.ok) {{
                    const data = await response.json();
                    sourceCache[cacheKey] = data;
                    displaySource(data, line);
                }} else {{
                    document.getElementById('source-code').innerHTML = '<code class="text-gray-500">// Source not available</code>';
                }}
            }} catch (e) {{
                document.getElementById('source-code').innerHTML = '<code class="text-gray-500">// Error loading source</code>';
            }}
        }}
        function escapeHtml(text) {{
            if (!text) return '';
            return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#039;');
        }}
        function displaySource(data, highlightLine) {{
            let html = '';
            const start = Math.max(1, data.line - 5);
            const end = data.line + 5;
            for (let i = start; i <= end; i++) {{
                const isErrorLine = i === data.line;
                const lineNum = String(i).padStart(4, ' ');
                const lineClass = isErrorLine ? 'bg-red-500/20 text-red-300' : 'text-gray-400';
                const bgClass = isErrorLine ? 'bg-red-500/10' : '';
                const escapedLine = escapeHtml(data.lines[i] || '');
                html += '<tr class="' + bgClass + '"><td class="text-gray-600 select-none pr-2">' + lineNum + '</td><td class="' + lineClass + '"><pre style="margin:0;white-space:pre-wrap;">' + escapedLine + '</pre></td></tr>';
            }}
            document.getElementById('source-code').innerHTML = '<table class="w-full">' + html + '</table>';
        }}
        async function executeRepl() {{
            const input = document.getElementById('repl-input');
            let code = input.value.trim();
            if (!code) return;
            if (lastResult !== null) {{ code = code.replace(/@/g, '(' + lastResult + ')'); }}
            if (code && history[history.length - 1] !== code) {{ history.push(code); }}
            historyIndex = history.length;
            const output = document.getElementById('repl-output');
            output.innerHTML += '<div class="flex items-center gap-2 text-gray-400 mt-2"><div class="loading-spinner"></div><span>Executing...</span></div>';
            output.scrollTop = output.scrollHeight;
            try {{
                const response = await fetch('/__dev/repl', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{ code: code, request_data: currentRequestData, breakpoint_env: breakpointEnv }})
                }});
                const result = await response.json();
                if (result.error) {{
                    output.innerHTML += '<div class="text-red-400 mt-2">❌ ' + escapeHtml(result.error) + '</div>';
                    lastResult = null;
                }} else {{
                    output.innerHTML += '<div class="text-gray-300 mt-2"><span class="text-indigo-400">❯</span> <span class="text-gray-500">// ' + escapeHtml(input.value.trim()) + '</span></div>';
                    if (result.result && result.result !== "ok") {{
                        output.innerHTML += '<div class="text-green-400 mt-1">' + escapeHtml(result.result) + '</div>';
                        lastResult = result.result;
                    }} else {{
                        lastResult = null;
                    }}
                }}
            }} catch (e) {{
                output.innerHTML += '<div class="text-red-400 mt-2">❌ Error: ' + escapeHtml(e.message) + '</div>';
                lastResult = null;
            }}
            output.scrollTop = output.scrollHeight;
            input.value = '';
        }}
        let history = [];
        let historyIndex = -1;
        let lastResult = null;
        function navigateHistory(direction) {{
            const input = document.getElementById('repl-input');
            if (history.length === 0) return;
            if (direction === 'up') {{
                if (historyIndex > 0) {{ historyIndex--; input.value = history[historyIndex]; }}
            }} else if (direction === 'down') {{
                if (historyIndex < history.length - 1) {{ historyIndex++; input.value = history[historyIndex]; }}
                else {{ historyIndex = history.length; input.value = ''; }}
            }}
            const length = input.value.length;
            setTimeout(() => {{ input.setSelectionRange(length, length); }}, 0);
        }}
        function quickInspect(expr) {{
            let expanded = expr;
            if (lastResult !== null) {{ expanded = expr.replace(/@/g, '(' + lastResult + ')'); }}
            document.getElementById('repl-input').value = expanded;
            executeRepl();
        }}
        function clearRepl() {{
            document.getElementById('repl-output').innerHTML = '<div class="text-gray-500 italic">// REPL cleared.</div>';
            history = [];
            historyIndex = -1;
            lastResult = null;
        }}
        function formatJson(obj, indent = 0) {{
            const spaces = '  '.repeat(indent);
            const nextSpaces = '  '.repeat(indent + 1);
            if (obj === null) return '<span class="text-orange-400">null</span>';
            if (obj === undefined) return '<span class="text-gray-500">undefined</span>';
            if (typeof obj === 'boolean') return '<span class="text-orange-400">' + obj + '</span>';
            if (typeof obj === 'number') return '<span class="text-purple-400">' + obj + '</span>';
            if (typeof obj === 'string') return '<span class="text-green-400">"' + escapeHtml(obj) + '"</span>';
            if (Array.isArray(obj)) {{
                if (obj.length === 0) return '<span class="text-gray-400">[]</span>';
                let result = '<span class="text-gray-400">[</span>\n';
                obj.forEach((item, i) => {{ result += nextSpaces + formatJson(item, indent + 1); if (i < obj.length - 1) result += '<span class="text-gray-400">,</span>'; result += '\n'; }});
                result += spaces + '<span class="text-gray-400">]</span>';
                return result;
            }}
            if (typeof obj === 'object') {{
                const keys = Object.keys(obj);
                if (keys.length === 0) return '<span class="text-gray-400">{{}}</span>';
                let result = '<span class="text-gray-400">{{</span>\n';
                keys.forEach((key, i) => {{ result += nextSpaces + '<span class="text-indigo-300">"' + escapeHtml(key) + '"</span><span class="text-gray-400">:</span> ' + formatJson(obj[key], indent + 1); if (i < keys.length - 1) result += '<span class="text-gray-400">,</span>'; result += '\n'; }});
                result += spaces + '<span class="text-gray-400">}}</span>';
                return result;
            }}
            return '<span class="text-gray-400">' + escapeHtml(String(obj)) + '</span>';
        }}
        function initJsonDisplays() {{
            const displays = {{ 'json-params': currentRequestData.params, 'json-query': currentRequestData.query, 'json-body': currentRequestData.body, 'json-headers': currentRequestData.headers, 'json-session': currentRequestData.session }};
            for (const [id, data] of Object.entries(displays)) {{
                const el = document.getElementById(id);
                if (el) {{
                    if (data === null || data === undefined) el.innerHTML = '<span class="text-gray-500 italic">No data</span>';
                    else if (typeof data === 'object' && Object.keys(data).length === 0) el.innerHTML = '<span class="text-gray-500 italic">Empty</span>';
                    else el.innerHTML = formatJson(data);
                }}
            }}
        }}
        initJsonDisplays();
        document.addEventListener('keydown', function(e) {{
            if (e.ctrlKey && e.key === '`') {{ e.preventDefault(); document.getElementById('repl-input').focus(); }}
            if (e.key === 'Escape') {{ document.querySelectorAll('.stack-frame').forEach(el => el.classList.remove('active')); }}
            if (e.target.id === 'repl-input') {{
                if (e.key === 'ArrowUp') {{ e.preventDefault(); navigateHistory('up'); }}
                else if (e.key === 'ArrowDown') {{ e.preventDefault(); navigateHistory('down'); }}
            }}
        }});
        document.addEventListener('DOMContentLoaded', function() {{
            const frames = document.querySelectorAll('.stack-frame');
            if (frames.length > 0) frames[frames.length - 1].click();
            document.getElementById('repl-input').focus();
        }});
    </script>
</body>
</html>"#,
        error_type = error_type,
        error_message = error_message,
        error_location = error_location,
        stack_frames = stack_frames.join("\n"),
        request_data_json = escape_for_script_tag(request_data_json),
        breakpoint_env_js = escape_for_script_tag(breakpoint_env_json.unwrap_or("null")),
        preloaded_sources_js = escape_for_script_tag(&preloaded_sources_js),
        request_method = escape_html(&request_method),
        request_path = escape_html(&request_path),
        request_time = request_time,
    )
}

fn extract_controller_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn escape_for_script_tag(s: &str) -> String {
    s.replace("</", "<\\/")
        .replace("<!--", "<\\!--")
        .replace("]]>", "]]\\>")
}

#[allow(dead_code)]
pub(super) fn get_source_file(
    file_path: &str,
    _line: usize,
) -> Option<HashMap<String, HashMap<usize, String>>> {
    let path = Path::new(file_path);
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let lines: HashMap<usize, String> = content
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.to_string()))
        .collect();
    Some([(file_path.to_string(), lines)].iter().cloned().collect())
}

pub(super) fn render_production_error_page(
    status_code: u16,
    message: &str,
    request_id: &str,
) -> String {
    if let Some(custom_html) = crate::interpreter::builtins::template::render_error_template(
        status_code,
        message,
        request_id,
    ) {
        return custom_html;
    }

    let (title, heading, description, code_class) = match status_code {
        400 => (
            "400 Bad Request".to_string(),
            "Bad Request".to_string(),
            "The request could not be understood by the server due to malformed syntax."
                .to_string(),
            "warning".to_string(),
        ),
        401 => (
            "401 Unauthorized".to_string(),
            "Authentication Required".to_string(),
            "You need to sign in to access this resource.".to_string(),
            "warning".to_string(),
        ),
        403 => (
            "403 Forbidden".to_string(),
            "Forbidden".to_string(),
            "You don't have permission to access this resource.".to_string(),
            "warning".to_string(),
        ),
        404 => (
            "404 Not Found".to_string(),
            "Page Not Found".to_string(),
            "The page you're looking for doesn't exist or has been moved.".to_string(),
            "warning".to_string(),
        ),
        405 => (
            "405 Method Not Allowed".to_string(),
            "Method Not Allowed".to_string(),
            "The HTTP method used is not allowed for this resource.".to_string(),
            "warning".to_string(),
        ),
        409 => (
            "409 Conflict".to_string(),
            "Conflict".to_string(),
            "This request conflicts with the current state of the resource. Reload and try again."
                .to_string(),
            "warning".to_string(),
        ),
        410 => (
            "410 Gone".to_string(),
            "Gone".to_string(),
            "This resource used to exist but has been permanently removed.".to_string(),
            "warning".to_string(),
        ),
        422 => (
            "422 Unprocessable Entity".to_string(),
            "Unprocessable Entity".to_string(),
            "The request was well-formed but could not be processed due to validation errors."
                .to_string(),
            "warning".to_string(),
        ),
        429 => (
            "429 Too Many Requests".to_string(),
            "Too Many Requests".to_string(),
            "You've sent too many requests too quickly. Please slow down and try again shortly."
                .to_string(),
            "warning".to_string(),
        ),
        500 => (
            "500 Internal Server Error".to_string(),
            "Internal Server Error".to_string(),
            "Something went wrong on our end. Please try again later.".to_string(),
            "error".to_string(),
        ),
        501 => (
            "501 Not Implemented".to_string(),
            "Not Implemented".to_string(),
            "This feature isn't available yet.".to_string(),
            "error".to_string(),
        ),
        502 => (
            "502 Bad Gateway".to_string(),
            "Bad Gateway".to_string(),
            "The server received an invalid response from the upstream server.".to_string(),
            "error".to_string(),
        ),
        503 => (
            "503 Service Unavailable".to_string(),
            "Service Unavailable".to_string(),
            "The service is temporarily unavailable. Please try again later.".to_string(),
            "error".to_string(),
        ),
        504 => (
            "504 Gateway Timeout".to_string(),
            "Gateway Timeout".to_string(),
            "An upstream server didn't respond in time. Please try again.".to_string(),
            "error".to_string(),
        ),
        _ => {
            let status_text = get_status_text(status_code).to_string();
            (
                format!("{} {}", status_code, status_text),
                status_text.clone(),
                "An error occurred while processing your request.".to_string(),
                "info".to_string(),
            )
        }
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif; background-color: #f8f9fa; color: #212529; min-height: 100vh; display: flex; align-items: center; justify-content: center; padding: 20px; }}
        .container {{ text-align: center; max-width: 500px; }}
        .error-code {{ font-size: 120px; font-weight: 700; color: #e9ecef; line-height: 1; margin-bottom: 20px; }}
        .error-code.error {{ color: #f8d7da; }}
        .error-code.warning {{ color: #856404; }}
        .error-code.info {{ color: #d1ecf1; }}
        h1 {{ font-size: 28px; font-weight: 600; color: #343a40; margin-bottom: 12px; }}
        p {{ font-size: 16px; color: #6c757d; line-height: 1.6; margin-bottom: 24px; }}
        .actions {{ display: flex; gap: 12px; justify-content: center; flex-wrap: wrap; }}
        .btn {{ display: inline-flex; align-items: center; padding: 12px 24px; font-size: 14px; font-weight: 500; text-decoration: none; border-radius: 6px; cursor: pointer; transition: all 0.2s ease; }}
        .btn-primary {{ background-color: #007bff; color: white; border: none; }}
        .btn-primary:hover {{ background-color: #0056b3; }}
        .btn-secondary {{ background-color: transparent; color: #6c757d; border: 1px solid #dee2e6; }}
        .btn-secondary:hover {{ background-color: #f8f9fa; border-color: #adb5bd; }}
        .error-details {{ margin-top: 32px; padding-top: 24px; border-top: 1px solid #dee2e6; font-size: 12px; color: #adb5bd; }}
        .error-id {{ font-family: monospace; background: #e9ecef; padding: 2px 6px; border-radius: 4px; font-size: 11px; }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-code {code_class}">{status_code}</div>
        <h1>{heading}</h1>
        <p>{description}</p>
        <div class="actions">
            <a href="/" class="btn btn-primary">Go to Homepage</a>
            <button onclick="history.back()" class="btn btn-secondary">Go Back</button>
        </div>
        <div class="error-details">
            <p>If this problem persists, please contact support with the error ID below.</p>
            <p style="margin-top: 8px;">Error ID: <span class="error-id">{request_id}</span></p>
        </div>
    </div>
</body>
</html>"#,
        title = escape_html(&title),
        status_code = status_code,
        heading = escape_html(&heading),
        description = escape_html(&description),
        code_class = code_class,
        request_id = request_id,
    )
}

fn get_status_text(status_code: u16) -> &'static str {
    match status_code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Production error-page arms cover the statuses real Soli apps actually
    // emit: auth (401/403), missing (404/410), client bugs (400/405/409/422/429),
    // and server bugs (500/501/502/503/504). Any of these regressing to the
    // generic fallback is a UX regression worth catching.
    #[test]
    fn client_error_statuses_render_with_their_own_heading_and_warning_style() {
        let cases = [
            (400, "Bad Request"),
            (401, "Authentication Required"),
            (403, "Forbidden"),
            (404, "Page Not Found"),
            (405, "Method Not Allowed"),
            (409, "Conflict"),
            (410, "Gone"),
            (422, "Unprocessable Entity"),
            (429, "Too Many Requests"),
        ];
        for (code, heading) in cases {
            let html = render_production_error_page(code, "msg", "req-id");
            assert!(
                html.contains(heading),
                "status {}: heading {:?} missing from page",
                code,
                heading
            );
            assert!(
                html.contains("error-code warning"),
                "status {}: expected `warning` CSS class on the big number",
                code
            );
            assert!(
                html.contains(&format!(">{}</div>", code)),
                "status {}: big number missing from page",
                code
            );
        }
    }

    #[test]
    fn server_error_statuses_render_with_their_own_heading_and_error_style() {
        let cases = [
            (500, "Internal Server Error"),
            (501, "Not Implemented"),
            (502, "Bad Gateway"),
            (503, "Service Unavailable"),
            (504, "Gateway Timeout"),
        ];
        for (code, heading) in cases {
            let html = render_production_error_page(code, "msg", "req-id");
            assert!(
                html.contains(heading),
                "status {}: heading {:?} missing from page",
                code,
                heading
            );
            assert!(
                html.contains("error-code error"),
                "status {}: expected `error` CSS class on the big number",
                code
            );
        }
    }

    // A status code outside the first-class list still produces a sensible
    // page with the right heading and the generic `info` style.
    #[test]
    fn unknown_status_falls_through_to_generic_info_page() {
        let html = render_production_error_page(418, "msg", "req-id");
        // Unknown-to-the-explicit-match but get_status_text still covers 418?
        // Actually 418 isn't in get_status_text, so the heading is "Error".
        assert!(html.contains("error-code info"));
    }
}
