//! User generics: `class Box<T>`, `def first<T>`, `Box<Int>` (#693, phase 5).
//!
//! Type parameters are erased at run time — both engines run a generic class
//! as any other — and `soli check` binds them: from a constructor's
//! arguments (`Box.new(1)` is a `Box<Int>`), from a call's (`first([1])` is
//! an `Int`), or from an annotation (`let b: Box<String>`).

use std::process::Command;

const BOX: &str = "class Box<T>
  new(item: T)
    @item = item
  end

  def get -> T
    @item
  end

  def map<U>(f: (T) -> U) -> Box<U>
    Box.new(f(@item))
  end
end

class Point
  def norm -> Int
    1
  end
end

def first<T>(xs: T[]) -> T
  xs[0]
end
";

fn check(source: &str) -> Result<(), String> {
    solilang::type_check_source(source, None)
        .map(|_| ())
        .map_err(|errors| {
            errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })
}

fn run(source: &str, engine: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("g.sl");
    std::fs::write(&file, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg(engine)
        .arg(&file)
        .output()
        .expect("run soli");
    assert!(
        output.status.success(),
        "{engine}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_constructor_binds_the_class_parameters() {
    let err = check(&format!("{BOX}b = Box.new(Point.new)\nb.get.nrom\n")).unwrap_err();
    assert!(err.contains("'nrom' on type 'Point'"), "{err}");
    check(&format!("{BOX}b = Box.new(Point.new)\nprint(b.get.norm)\n")).unwrap();
}

#[test]
fn an_annotation_binds_them_too() {
    let err = check(&format!("{BOX}let s: Box<String> = Box.new(3)\n")).unwrap_err();
    assert!(
        err.contains("expected Box<String>, found Box<Int>"),
        "{err}"
    );
    check(&format!(
        "{BOX}let s: Box<String> = Box.new(\"a\")\nprint(s.get)\n"
    ))
    .unwrap();
}

#[test]
fn a_call_binds_a_generic_def() {
    let err = check(&format!("{BOX}let n: Int = first([\"a\"])\n")).unwrap_err();
    assert!(err.contains("expected Int, found String"), "{err}");
    let err = check(&format!("{BOX}first([Point.new]).nrom\n")).unwrap_err();
    assert!(err.contains("'nrom' on type 'Point'"), "{err}");
    check(&format!("{BOX}let n: Int = first([1, 2])\n")).unwrap();
}

#[test]
fn a_generic_method_of_a_generic_class() {
    check(&format!(
        "{BOX}b = Box.new(1)\nlet s: Box<String> = b.map(fn(x) {{ x.to_s }})\n"
    ))
    .unwrap();
}

#[test]
fn an_unbound_parameter_is_open() {
    // Nothing binds `T`: the value read is unchecked, as `Any`.
    check(&format!(
        "{BOX}def wrap(x)\n  Box.new(x)\nend\nprint(wrap(1).get.anything)\n"
    ))
    .unwrap();
}

#[test]
fn hash_and_array_take_type_arguments() {
    check("let counts: Hash<String, Int> = {\"a\": 1}\nlet xs: Array<Int> = [1]\n").unwrap();
    let err = check("let xs: Array<Int> = [\"a\"]\n").unwrap_err();
    assert!(err.contains("expected Int[]"), "{err}");
}

#[test]
fn class_b_lt_a_is_still_inheritance() {
    let source = "class A\n  def hi\n    \"hi\"\n  end\nend\nclass B<A\nend\nprint(B.new.hi)\n";
    check(source).unwrap();
    for engine in ["--vm", "--tree"] {
        assert_eq!(run(source, engine).trim(), "hi");
    }
}

#[test]
fn both_engines_run_a_generic_class_erased() {
    let source = format!(
        "{BOX}class Base\nend\nclass Pair<K, V> < Base\n  new(key: K, value: V)\n    @key = key\n  end\n\n  def key -> K\n    @key\n  end\nend\nb = Box.new(41)\nprint(b.get + 1)\nprint(b.map(fn(x) {{ x * 2 }}).get)\nprint(first([3, 4]))\nprint(Pair.new(\"k\", 2).key)\n"
    );
    for engine in ["--vm", "--tree"] {
        assert_eq!(run(&source, engine), "42\n82\n3\nk\n", "{engine}");
    }
}

#[test]
fn fmt_keeps_type_parameters() {
    let formatted = solilang::fmt::format_source(BOX).expect("formats");
    assert!(formatted.contains("class Box<T>"), "{formatted}");
    assert!(
        formatted.contains("def map<U>(f: (T) -> U) -> Box<U>"),
        "{formatted}"
    );
    assert!(
        formatted.contains("def first<T>(xs: T[]) -> T"),
        "{formatted}"
    );
}
