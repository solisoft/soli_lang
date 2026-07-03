//! End-to-end for the single-source path: a typed `Invoice` drives both the
//! visual PDF and the embedded EN 16931 CII XML, and the result is a
//! structurally valid PDF/A-3b Factur-X file whose XML round-trips.

use std::time::Duration;

use lopdf::Object;
use soli_pdf::{
    facturx, generate_facturx_from_invoice, FacturxMetadata, Invoice, Profile, RenderOptions,
};

const TEMPLATE: &[u8] = include_bytes!("fixtures/template.json");
const INVOICE: &[u8] = include_bytes!("fixtures/invoice.json");

fn offline_opts() -> RenderOptions {
    RenderOptions {
        fetch_images: false,
        http_timeout: Duration::from_secs(1),
        font_dirs: vec!["fonts".into()],
        ..Default::default()
    }
}

#[test]
fn invoice_drives_pdf_and_consistent_xml() {
    let invoice = Invoice::parse(INVOICE).expect("parse invoice");

    // The generated XML carries the totals computed from the lines.
    let xml = invoice.to_cii_xml(Profile::En16931).expect("cii xml");
    assert!(xml.contains("<ram:GrandTotalAmount>600.00</ram:GrandTotalAmount>"));
    assert!(xml.contains("<ram:DuePayableAmount>600.00</ram:DuePayableAmount>"));

    // The same totals appear on the visual side (its render data).
    let data = invoice.to_render_data();
    assert_eq!(data["data"]["total"]["due_amount"], "€600.00");
    assert_eq!(data["data"]["invoice"]["number"], "#12345");

    let pdf = generate_facturx_from_invoice(
        TEMPLATE,
        &invoice,
        Profile::En16931,
        &FacturxMetadata::default(),
        &offline_opts(),
    )
    .expect("generate");

    let doc = lopdf::Document::load_mem(&pdf).expect("reparse");
    assert_eq!(doc.version, "1.7", "PDF/A-3 must be PDF 1.7");

    let catalog = doc.catalog().expect("catalog");
    assert!(catalog.has(b"AF"), "catalog has /AF");
    assert!(catalog.has(b"Names"), "catalog has /Names");
    assert!(catalog.has(b"OutputIntents"), "catalog has /OutputIntents");
    assert!(catalog.has(b"Metadata"), "catalog has /Metadata");

    // The embedded factur-x.xml is exactly the XML we generated, and it parses.
    let embedded = find_embedded_file(&doc).expect("embedded file present");
    assert_eq!(
        embedded,
        xml.as_bytes(),
        "embedded XML matches generated XML"
    );
    let embedded_str = String::from_utf8(embedded).expect("utf8 xml");
    assert!(embedded_str.contains("<rsm:CrossIndustryInvoice"));
    assert!(embedded_str.contains("urn:cen.eu:en16931:2017"));
}

#[test]
fn embed_consistency_check_against_caller_xml() {
    // A caller using their own XML can still reuse the generator to cross-check.
    let invoice = Invoice::parse(INVOICE).unwrap();
    let xml = invoice.to_cii_xml(Profile::En16931).unwrap();

    // The free-form path embeds the same bytes and they round-trip.
    let rendered = soli_pdf::render_with_warnings(
        TEMPLATE,
        &serde_json::to_vec(&invoice.to_render_data()).unwrap(),
        &offline_opts(),
    )
    .unwrap();
    let pdf = facturx::embed_facturx(
        &rendered.pdf,
        xml.as_bytes(),
        Profile::En16931,
        &FacturxMetadata::default(),
    )
    .unwrap();
    let doc = lopdf::Document::load_mem(&pdf).unwrap();
    assert_eq!(find_embedded_file(&doc).unwrap(), xml.as_bytes());
}

#[test]
fn enriched_invoice_embeds_adjusted_totals() {
    // Enrich the fixture invoice with an allowance, a charge and payment terms;
    // the embedded XML must carry the adjusted BT-109 chain.
    let mut invoice: serde_json::Value = serde_json::from_slice(INVOICE).unwrap();
    invoice["allowances"] = serde_json::json!([
        { "reason": "Volume discount", "percent": 10, "vat_rate": 20.0 }
    ]);
    invoice["charges"] = serde_json::json!([
        { "reason": "Shipping", "amount": "20.00", "vat_rate": 20.0 }
    ]);
    invoice["payment_terms"] = serde_json::json!("30 days net");
    let invoice = Invoice::parse(&serde_json::to_vec(&invoice).unwrap()).expect("parse enriched");

    let xml = invoice.to_cii_xml(Profile::En16931).expect("cii xml");
    // Lines total 500; -10% +20 → 470 basis, 94 VAT, 564 due.
    assert!(xml.contains("<ram:TaxBasisTotalAmount>470.00</ram:TaxBasisTotalAmount>"));
    assert!(xml.contains("<ram:AllowanceTotalAmount>50.00</ram:AllowanceTotalAmount>"));
    assert!(xml.contains("<ram:ChargeTotalAmount>20.00</ram:ChargeTotalAmount>"));
    assert!(xml.contains("<ram:GrandTotalAmount>564.00</ram:GrandTotalAmount>"));
    assert!(xml.contains("<ram:Description>30 days net</ram:Description>"));

    let pdf = generate_facturx_from_invoice(
        TEMPLATE,
        &invoice,
        Profile::En16931,
        &FacturxMetadata::default(),
        &offline_opts(),
    )
    .expect("generate enriched");
    let doc = lopdf::Document::load_mem(&pdf).expect("reparse");
    let embedded = find_embedded_file(&doc).expect("embedded file present");
    assert_eq!(
        embedded,
        xml.as_bytes(),
        "embedded XML matches generated XML"
    );
}

fn find_embedded_file(doc: &lopdf::Document) -> Option<Vec<u8>> {
    for (_, obj) in doc.objects.iter() {
        if let Object::Stream(s) = obj {
            if let Ok(Object::Name(t)) = s.dict.get(b"Type") {
                if t == b"EmbeddedFile" {
                    return Some(s.content.clone());
                }
            }
        }
    }
    None
}
