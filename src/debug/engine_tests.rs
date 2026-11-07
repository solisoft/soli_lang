//! Both engines feed the hooks: the dev flamegraph sees VM functions as it
//! sees tree-walked ones, and the profiler names the hot function on each.

use crate::serve::span_log::{self, SpanKind};
use crate::Engine;

const FLAME_SCRIPT: &str = r#"
def helper(n)
  s = ""
  n.times do |i|
    s = s + "x"
  end
  s
end

def outer
  helper(3) + helper(2)
end

outer()
"#;

/// The `fn` span names recorded while `source` runs on `engine`.
fn fn_spans(engine: Engine) -> Vec<(String, Option<String>)> {
    std::thread::spawn(move || {
        span_log::set_enabled(true);
        span_log::begin_request(web_time::Instant::now());
        crate::run_script(FLAME_SCRIPT, None, false, engine).expect("script runs");
        let spans: Vec<(String, Option<String>)> = span_log::snapshot()
            .into_iter()
            .filter(|span| span.kind == SpanKind::Fn)
            .map(|span| (span.name, span.parent.map(|p| p.to_string())))
            .collect();
        span_log::set_enabled(false);
        assert_eq!(super::depth(), 0, "every frame was closed");
        spans
    })
    .join()
    .unwrap()
}

#[test]
fn flamegraph_sees_user_functions_on_both_engines() {
    for engine in [Engine::Vm, Engine::Tree] {
        let spans = fn_spans(engine);
        let names: Vec<&str> = spans.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names.iter().filter(|n| **n == "outer").count(),
            1,
            "{engine:?}: {names:?}"
        );
        assert_eq!(
            names.iter().filter(|n| **n == "helper").count(),
            2,
            "{engine:?}: {names:?}"
        );
        // `helper` nests inside `outer`.
        assert!(
            spans
                .iter()
                .filter(|(name, _)| name == "helper")
                .all(|(_, parent)| parent.is_some()),
            "{engine:?}: {spans:?}"
        );
    }
}

fn burn_script(iterations: u32) -> String {
    format!(
        r#"
def burn(n)
  total = 0
  i = 0
  while i < n
    total = total + i % 7
    i = i + 1
  end
  total
end

def outer
  burn({iterations})
end

outer()
"#
    )
}

#[test]
fn profiler_names_the_hot_function_on_both_engines() {
    let _serial = super::profiler::TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for (engine, iterations) in [(Engine::Vm, 3_000_000), (Engine::Tree, 300_000)] {
        let report = std::thread::spawn(move || {
            let profiler = super::profiler::start(1000).expect("no other profile runs");
            crate::run_script(&burn_script(iterations), None, false, engine).expect("script runs");
            profiler.finish()
        })
        .join()
        .unwrap();
        let folded = report.to_folded();
        // The profiler samples every engine thread of the process, so other
        // tests running beside this one show up too: judge only the stacks
        // under `outer`.
        let id_of = |name: &str| {
            report
                .frames
                .iter()
                .position(|frame| frame.name == name)
                .unwrap_or_else(|| panic!("{engine:?}: {name} never sampled\n{folded}"))
                as u32
        };
        let (outer, burn) = (id_of("outer"), id_of("burn"));
        let under_outer: Vec<&(Vec<u32>, u64)> = report
            .stacks
            .iter()
            .filter(|(ids, _)| ids.get(1) == Some(&outer))
            .collect();
        let total: u64 = under_outer.iter().map(|(_, ticks)| ticks).sum();
        let in_burn: u64 = under_outer
            .iter()
            .filter(|(ids, _)| ids.last() == Some(&burn))
            .map(|(_, ticks)| ticks)
            .sum();
        assert!(total >= 5, "{engine:?}: too few samples\n{folded}");
        assert!(
            in_burn * 10 >= total * 8,
            "{engine:?}: burn should hold most of the time\n{folded}"
        );
    }
}

#[test]
fn an_uncaught_error_leaves_no_frame_open_on_either_engine() {
    for engine in [Engine::Vm, Engine::Tree] {
        std::thread::spawn(move || {
            span_log::set_enabled(true);
            span_log::begin_request(web_time::Instant::now());
            let script = r#"
def fail_deep
  throw "boom"
end

def outer
  fail_deep()
end

outer()
"#;
            assert!(crate::run_script(script, None, false, engine).is_err());
            assert_eq!(
                super::depth(),
                0,
                "{engine:?}: the frames the error unwound past are closed"
            );
            span_log::set_enabled(false);
        })
        .join()
        .unwrap();
    }
}
