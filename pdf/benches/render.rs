//! Benchmarks for the parse → render → Factur-X pipeline.
//!
//! Run with `cargo bench`. Each benchmark uses the request fixtures and keeps
//! image fetching off so results are deterministic and offline.

use std::path::PathBuf;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use soli_pdf::data::DataDocument;
use soli_pdf::fonts::FontRegistry;
use soli_pdf::template::Template;
use soli_pdf::{facturx, render_to_bytes, FacturxMetadata, Profile, RenderOptions};

const TEMPLATE: &[u8] = include_bytes!("../tests/fixtures/template.json");
const DATA: &[u8] = include_bytes!("../tests/fixtures/data.json");
const XML: &[u8] = include_bytes!("../tests/fixtures/factur-x.xml");

fn opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
        ..Default::default()
    }
}

fn opts_cjk() -> RenderOptions {
    // Includes the CJK fallback font for the with-CJK render benchmark.
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into(), "../font".into()],
        ..Default::default()
    }
}

fn bench_parse(c: &mut Criterion) {
    c.bench_function("parse_template", |b| {
        b.iter(|| Template::parse(std::hint::black_box(TEMPLATE)).unwrap())
    });
    c.bench_function("parse_data", |b| {
        b.iter(|| DataDocument::parse(std::hint::black_box(DATA)).unwrap())
    });
}

fn bench_fonts(c: &mut Criterion) {
    // Loading + parsing the font directory (Latin family from ./fonts).
    let dirs = [PathBuf::from("fonts")];
    let fams = ["titillium".to_string()];
    c.bench_function("font_registry_load", |b| {
        b.iter(|| FontRegistry::from_font_dirs(&dirs, &fams).unwrap())
    });
}

fn bench_render(c: &mut Criterion) {
    // With the CJK fallback present, the sample's title embeds the (large) CJK
    // font — the heavy case.
    let o_cjk = opts_cjk();
    c.bench_function("render_to_bytes_cjk", |b| {
        b.iter(|| {
            render_to_bytes(
                std::hint::black_box(TEMPLATE),
                std::hint::black_box(DATA),
                &o_cjk,
            )
            .unwrap()
        })
    });

    // Latin-only variant: the common case. No CJK font is loaded/embedded, so
    // this is far faster and produces a ~100 KB PDF.
    let o = opts();
    let latin = String::from_utf8(TEMPLATE.to_vec())
        .unwrap()
        .replace("こんにちは世界", "World");
    let latin_bytes = latin.into_bytes();
    c.bench_function("render_to_bytes_latin", |b| {
        b.iter(|| {
            render_to_bytes(
                std::hint::black_box(&latin_bytes),
                std::hint::black_box(DATA),
                &o,
            )
            .unwrap()
        })
    });
}

fn bench_facturx(c: &mut Criterion) {
    let o = opts();
    let pdf = render_to_bytes(TEMPLATE, DATA, &o).unwrap();
    let meta = FacturxMetadata::default();
    c.bench_function("embed_facturx", |b| {
        b.iter(|| {
            facturx::embed_facturx(
                std::hint::black_box(&pdf),
                std::hint::black_box(XML),
                Profile::En16931,
                &meta,
            )
            .unwrap()
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20).measurement_time(Duration::from_secs(8));
    targets = bench_parse, bench_fonts, bench_render, bench_facturx
}
criterion_main!(benches);
