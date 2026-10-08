//! Microbench for rendering a parsed template (`render_nodes`), no HTTP.
//!
//! `rows50` is the gap bench's `/view` page: a 50-row table, three output
//! expressions per row. The other cases keep the loop and one kind of
//! expression each, so a change to one evaluation path shows up on its own:
//! a hash index, a paren-free method call, arithmetic, a bare local, and the
//! literal-only loop that is the floor under all of them.

use std::cell::RefCell;
use std::rc::Rc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use solilang::interpreter::value::{HashKey, HashPairs, Value};
use solilang::template::parser::parse_template;
use solilang::template::renderer::render_nodes;

fn rows(n: i64) -> Value {
    let items = (1..=n)
        .map(|i| {
            let mut row = HashPairs::default();
            row.insert(HashKey::String("id".into()), Value::Int(i));
            row.insert(
                HashKey::String("title".into()),
                Value::String(format!("Post title {i}").into()),
            );
            row.insert(HashKey::String("views".into()), Value::Int(i * 7));
            Value::Hash(Rc::new(RefCell::new(row)))
        })
        .collect();
    Value::Array(Rc::new(RefCell::new(items)))
}

fn data() -> Value {
    let mut map = HashPairs::default();
    map.insert(HashKey::String("rows".into()), rows(50));
    map.insert(HashKey::String("stamp".into()), Value::Float(1.5));
    Value::Hash(Rc::new(RefCell::new(map)))
}

const CASES: &[(&str, &str)] = &[
    (
        "rows50",
        "<h1>Posts <%= stamp %></h1>\n<table>\n<% for row in rows %>\n  <tr><td><%= row[\"id\"] %></td><td><%= row[\"title\"].upcase %></td><td><%= row[\"views\"] * 2 %></td></tr>\n<% end %>\n</table>\n",
    ),
    ("one_literal", "<p>x</p>"),
    ("literal", "<% for row in rows %><tr><td>x</td></tr><% end %>"),
    ("local", "<% for row in rows %><td><%= stamp %></td><% end %>"),
    ("index", "<% for row in rows %><td><%= row[\"id\"] %></td><% end %>"),
    ("index_str", "<% for row in rows %><td><%= row[\"title\"] %></td><% end %>"),
    ("method", "<% for row in rows %><td><%= row[\"title\"].upcase %></td><% end %>"),
    ("arith", "<% for row in rows %><td><%= row[\"views\"] * 2 %></td><% end %>"),
    ("dot", "<% for row in rows %><td><%= row.title %></td><% end %>"),
];

fn bench_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("template_render");
    for (name, source) in CASES {
        let nodes = parse_template(source).expect("template parses");
        let data = data();
        render_nodes(&nodes, &data, None).expect("template renders");
        group.bench_function(*name, |b| {
            b.iter(|| render_nodes(black_box(&nodes), black_box(&data), None).unwrap())
        });
    }
    group.finish();
}

criterion_group!(benches, bench_render);
criterion_main!(benches);
