//! Parse the real request fixtures and assert the model decoded as intended.

use soli_pdf::template::{Cell, CellContent, Element, Template};

const TEMPLATE: &[u8] = include_bytes!("fixtures/template.json");
const DATA: &[u8] = include_bytes!("fixtures/data.json");

#[test]
fn template_parses() {
    let t = Template::parse(TEMPLATE).expect("template parses");
    assert_eq!(t.fonts, vec!["titillium"]);
    assert_eq!(t.options.header_height, 0.0);
    assert_eq!(t.footer.len(), 1);
    // Footer paragraph keeps the page tokens (substituted later).
    match &t.footer[0] {
        Element::Paragraph(p) => assert!(p.value.contains("#PAGE#")),
        _ => panic!("footer[0] should be a paragraph"),
    }

    // The content has paragraphs, moves, an image and tables.
    let mut tables = 0;
    let mut images = 0;
    let mut moves = 0;
    let mut paras = 0;
    for el in &t.content {
        match el {
            Element::Table(_) => tables += 1,
            Element::Image(_) => images += 1,
            Element::Move(_) => moves += 1,
            Element::Paragraph(_) => paras += 1,
            _ => {}
        }
    }
    assert_eq!((tables, images, moves, paras), (4, 1, 10, 4));
}

#[test]
fn data_bound_table_and_rich_cells() {
    let t = Template::parse(TEMPLATE).unwrap();
    // The items table is the data-bound one.
    let items_table = t
        .content
        .iter()
        .find_map(|e| match e {
            Element::Table(tb) if tb.data.as_deref() == Some("items") => Some(tb),
            _ => None,
        })
        .expect("items table");
    assert_eq!(items_table.header_columns.len(), 4);
    assert_eq!(items_table.options.padding_y, 3.0);
    assert_eq!(
        items_table.options.header.fill_color.as_deref(),
        Some("fff")
    );

    // The company/customer table uses rich cells with stacked text content.
    let rich_table = t
        .content
        .iter()
        .filter_map(|e| match e {
            Element::Table(tb) => Some(tb),
            _ => None,
        })
        .find(|tb| matches!(tb.rows.first().and_then(|r| r.first()), Some(Cell::Rich(_))))
        .expect("a table with rich cells");
    match &rich_table.rows[0][0] {
        Cell::Rich(rc) => {
            assert_eq!(rc.content.len(), 5);
            assert!(matches!(rc.content[0], CellContent::Text(_)));
        }
        _ => panic!("expected rich cell"),
    }
}

#[test]
fn case_insensitive_alignment_and_stringy_bools() {
    use soli_pdf::template::Alignment;
    let t = Template::parse(TEMPLATE).unwrap();
    // The totals table uses "alignment": "Left" (capitalized).
    let totals = t
        .content
        .iter()
        .filter_map(|e| match e {
            Element::Table(tb) => Some(tb),
            _ => None,
        })
        .find(|tb| {
            tb.data.is_none()
                && tb
                    .rows
                    .iter()
                    .flatten()
                    .any(|c| matches!(c, Cell::Text(tc) if tc.text == "Total amount"))
        })
        .expect("totals table");

    let label = totals
        .rows
        .iter()
        .flatten()
        .find_map(|c| match c {
            Cell::Text(tc) if tc.text == "Total amount" => Some(tc),
            _ => None,
        })
        .unwrap();
    assert_eq!(label.style.alignment, Some(Alignment::Left));
    // borderSides present with bottom:false and top omitted -> top defaults true.
    let bs = label.style.border_sides.unwrap();
    assert!(!bs.bottom);
    assert!(bs.top);
}

#[test]
fn data_parses() {
    let d = soli_pdf::data::DataDocument::parse(DATA).unwrap();
    assert_eq!(
        d.template_uuid.as_deref(),
        Some("9ea01428-3628-4083-8cad-081572ac2758")
    );
    let r = d.resolver();
    assert_eq!(r.lookup("invoice.number").as_deref(), Some("#12345"));
    assert_eq!(r.lookup("company.name").as_deref(), Some("PDFx"));
    let items = d.array("items").unwrap();
    assert_eq!(items.len(), 2);
    let row = d.resolver().with_scope(&items[1]);
    assert_eq!(row.lookup("name").as_deref(), Some("Item 2"));
    assert_eq!(row.lookup("total_amount").as_deref(), Some("400"));
}
