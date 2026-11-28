//! Spreadsheet parsing and export built-in functions (CSV and Excel).
//!
//! Provides the Spreadsheet class for parsing and exporting spreadsheet files:
//! - Spreadsheet.csv(content) - Parse CSV string to array
//! - Spreadsheet.csv_file(path) - Parse CSV file to array
//! - Spreadsheet.excel(path, sheet?) - Parse one sheet of an Excel file to array
//! - Spreadsheet.excel_sheets(path) - Parse every sheet: hash of name => array
//! - Spreadsheet.to_csv(data) - Convert array to CSV string
//! - Spreadsheet.csv_write(data, path) - Write array to CSV file
//! - Spreadsheet.excel_write(data, path) - Write array (one sheet) or hash of
//!   sheet name => array (one sheet each) to an Excel file

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::rc::Rc;

use calamine::{Reader, Xlsx};
use csv::ReaderBuilder;
use std::io::Write;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashPairs, NativeFunction, Value};

fn parse_csv_content(content: &str) -> Result<Value, String> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| format!("Spreadsheet.csv() error reading headers: {}", e))?
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut rows: Vec<Value> = Vec::new();

    for result in reader.records() {
        let record = result.map_err(|e| format!("Spreadsheet.csv() error reading row: {}", e))?;
        let hash_pairs = headers
            .iter()
            .zip(record.iter())
            .map(|(k, v)| {
                let key = crate::interpreter::value::HashKey::String(k.clone().into());
                let value = if v.is_empty() {
                    Value::Null
                } else {
                    Value::String(v.to_string().into())
                };
                (key, value)
            })
            .collect::<HashPairs>();
        rows.push(Value::Hash(Rc::new(RefCell::new(hash_pairs))));
    }

    Ok(Value::Array(Rc::new(RefCell::new(rows))))
}

fn parse_csv_file(path: &str) -> Result<Value, String> {
    // Through the SEC-006 jail like every other path-taking builtin. `File.*`,
    // `slurp` and `Image.*` were confined; the spreadsheet readers and writers
    // opened whatever string they were handed, which is the same
    // user-influenced-path problem the jail exists for.
    let path =
        &crate::interpreter::builtins::file::resolve_readable_path(path, "Spreadsheet.csv_file")?;
    let file = File::open(path).map_err(|e| {
        format!(
            "Spreadsheet.csv_file() cannot open {}: {}",
            path.display(),
            e
        )
    })?;
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(BufReader::new(file));

    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| format!("Spreadsheet.csv_file() error reading headers: {}", e))?
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut rows: Vec<Value> = Vec::new();

    for result in reader.records() {
        let record =
            result.map_err(|e| format!("Spreadsheet.csv_file() error reading row: {}", e))?;
        let hash_pairs = headers
            .iter()
            .zip(record.iter())
            .map(|(k, v)| {
                let key = crate::interpreter::value::HashKey::String(k.clone().into());
                let value = if v.is_empty() {
                    Value::Null
                } else {
                    Value::String(v.to_string().into())
                };
                (key, value)
            })
            .collect::<HashPairs>();
        rows.push(Value::Hash(Rc::new(RefCell::new(hash_pairs))));
    }

    Ok(Value::Array(Rc::new(RefCell::new(rows))))
}

fn parse_excel_file(path: &str, sheet: Option<&str>) -> Result<Value, String> {
    let mut workbook = open_xlsx(path, "Spreadsheet.excel")?;
    let sheet_name = match sheet {
        Some(name) => {
            if !workbook.sheet_names().iter().any(|n| n == name) {
                return Err(format!(
                    "Spreadsheet.excel() {} has no sheet named \"{}\" (sheets: {})",
                    path,
                    name,
                    workbook.sheet_names().join(", ")
                ));
            }
            name.to_string()
        }
        None => workbook
            .sheet_names()
            .first()
            .cloned()
            .ok_or_else(|| "Spreadsheet.excel() file has no sheets".to_string())?,
    };
    read_sheet(&mut workbook, &sheet_name, "Spreadsheet.excel")
}

/// Every sheet of the workbook, in workbook order: name => rows.
fn parse_excel_sheets(path: &str) -> Result<Value, String> {
    let mut workbook = open_xlsx(path, "Spreadsheet.excel_sheets")?;
    let mut sheets = HashPairs::default();
    for name in workbook.sheet_names() {
        let rows = read_sheet(&mut workbook, &name, "Spreadsheet.excel_sheets")?;
        sheets.insert(
            crate::interpreter::value::HashKey::String(name.into()),
            rows,
        );
    }
    Ok(Value::Hash(Rc::new(RefCell::new(sheets))))
}

/// Open a workbook through the SEC-006 jail and `O_NOFOLLOW`, like `File.*`
/// and `Spreadsheet.csv_file`. `calamine::open_workbook` took the string as
/// given, so under `soli serve` a request that chose the path could read any
/// workbook the process could reach, or follow a planted symlink out of the
/// app root.
///
/// A workbook is untrusted input as soon as an app imports what its users
/// upload, so it is bounded before calamine sees it: the file itself
/// ([`max_workbook_bytes`]) and the sum of the sizes its zip directory
/// declares once inflated ([`max_inflated_workbook_bytes`]) — the parts
/// calamine reads into memory. A panic inside the parser becomes an error
/// (see [`guard_parser`]).
fn open_xlsx(path: &str, op: &str) -> Result<Xlsx<BufReader<File>>, String> {
    let resolved = crate::interpreter::builtins::file::resolve_readable_path(path, op)?;
    let mut file = crate::interpreter::builtins::file::open_for_read(&resolved, false)
        .map_err(|e| format!("{}() cannot open {}: {}", op, path, e))?;
    check_workbook_size(&mut file, op, path)?;
    guard_parser(op, || {
        Xlsx::new(BufReader::new(file)).map_err(|e| format!("{}() cannot open {}: {}", op, path, e))
    })
}

/// Default cap on a workbook file read by `Spreadsheet.excel*`: 50 MiB.
const DEFAULT_MAX_WORKBOOK_BYTES: u64 = 50 * 1024 * 1024;
/// Default cap on the inflated size of a workbook's parts: 512 MiB.
const DEFAULT_MAX_INFLATED_WORKBOOK_BYTES: u64 = 512 * 1024 * 1024;

fn env_bytes(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|&bytes| bytes > 0)
        .unwrap_or(default)
}

/// `SOLI_SPREADSHEET_MAX_BYTES`, default 50 MiB.
fn max_workbook_bytes() -> u64 {
    env_bytes("SOLI_SPREADSHEET_MAX_BYTES", DEFAULT_MAX_WORKBOOK_BYTES)
}

/// `SOLI_SPREADSHEET_MAX_INFLATED_BYTES`, default 512 MiB.
fn max_inflated_workbook_bytes() -> u64 {
    env_bytes(
        "SOLI_SPREADSHEET_MAX_INFLATED_BYTES",
        DEFAULT_MAX_INFLATED_WORKBOOK_BYTES,
    )
}

/// Refuse a workbook larger than the caps before it is parsed, then rewind.
/// The inflated total comes from the zip's central directory, which a hostile
/// file can understate; it stops the honest zip bomb, and the parser's own
/// bounds (calamine ≥ 0.38, RUSTSEC-2026-0436) stop the rest.
fn check_workbook_size(file: &mut File, op: &str, path: &str) -> Result<(), String> {
    use std::io::{Seek, SeekFrom};
    let len = file
        .metadata()
        .map_err(|e| format!("{}() cannot read {}: {}", op, path, e))?
        .len();
    let max = max_workbook_bytes();
    if len > max {
        return Err(format!(
            "{}() {} is {} bytes, over the {} byte limit (SOLI_SPREADSHEET_MAX_BYTES)",
            op, path, len, max
        ));
    }
    let inflated = guard_parser(op, || {
        let archive = zip::ZipArchive::new(&mut *file)
            .map_err(|e| format!("{}() {} is not an xlsx workbook: {}", op, path, e))?;
        // `None` when the total does not fit the type: over any cap.
        Ok(archive
            .decompressed_size()
            .map_or(u64::MAX, |size| u64::try_from(size).unwrap_or(u64::MAX)))
    })?;
    let max_inflated = max_inflated_workbook_bytes();
    if inflated > max_inflated {
        return Err(format!(
            "{}() {} inflates to {} bytes, over the {} byte limit (SOLI_SPREADSHEET_MAX_INFLATED_BYTES)",
            op, path, inflated, max_inflated
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("{}() cannot read {}: {}", op, path, e))?;
    Ok(())
}

/// Run a parsing step, turning a panic inside the zip or spreadsheet parser
/// into an error: a malformed upload must fail its own call, not the request
/// (or the job) around it.
fn guard_parser<T>(op: &str, parse: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(parse)).unwrap_or_else(|_| {
        Err(format!(
            "{}() could not parse the workbook: the file is malformed",
            op
        ))
    })
}

/// One sheet as an array of hashes keyed by its first row.
fn read_sheet(
    workbook: &mut Xlsx<BufReader<File>>,
    sheet_name: &str,
    caller: &str,
) -> Result<Value, String> {
    let range = guard_parser(caller, || {
        workbook
            .worksheet_range(sheet_name)
            .map_err(|e| format!("{}() error reading sheet: {}", caller, e))
    })?;

    let mut rows: Vec<Value> = Vec::new();
    let mut headers: Vec<String> = Vec::new();

    for (row_idx, row) in range.rows().enumerate() {
        if row_idx == 0 {
            headers = row.iter().map(|c| c.to_string()).collect();
            continue;
        }

        let hash_pairs = headers
            .iter()
            .zip(row.iter())
            .map(|(k, v)| {
                let key = crate::interpreter::value::HashKey::String(k.clone().into());
                let value = match v {
                    calamine::Data::Empty => Value::Null,
                    calamine::Data::String(s) => Value::String(s.clone().into()),
                    calamine::Data::Float(f) => Value::Float(*f),
                    calamine::Data::Int(i) => Value::Int(*i),
                    calamine::Data::Bool(b) => Value::Bool(*b),
                    calamine::Data::DateTime(dt) => Value::String(dt.to_string().into()),
                    calamine::Data::Error(e) => Value::String(format!("<error: {:?}>", e).into()),
                    calamine::Data::DateTimeIso(s) => Value::String(s.clone().into()),
                    calamine::Data::DurationIso(s) => Value::String(s.clone().into()),
                };
                (key, value)
            })
            .collect::<HashPairs>();

        rows.push(Value::Hash(Rc::new(RefCell::new(hash_pairs))));
    }

    Ok(Value::Array(Rc::new(RefCell::new(rows))))
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone().to_string(),
        Value::Decimal(d) => d.to_string(),
        Value::Array(arr) => {
            let items: Vec<String> = arr.borrow().iter().map(value_to_string).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Hash(hash) => {
            let pairs: Vec<String> = hash
                .borrow()
                .iter()
                .map(|(k, v)| format!("{}: {}", k, value_to_string(v)))
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        _ => value.to_string(),
    }
}

fn extract_headers_and_rows(
    data: &Rc<RefCell<Vec<Value>>>,
) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    extract_rows_for(data, None)
}

/// Headers and cell texts. With `columns`, those keys in that order (a key
/// missing from a row gives an empty cell); without, the first row's keys,
/// sorted.
fn extract_rows_for(
    data: &Rc<RefCell<Vec<Value>>>,
    columns: Option<Vec<String>>,
) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let data_ref = data.borrow();
    if data_ref.is_empty() {
        return Ok((columns.unwrap_or_default(), Vec::new()));
    }

    let first_row = data_ref.first().ok_or("Data array is empty")?;

    let headers = match (first_row, columns) {
        (_, Some(columns)) => columns,
        (Value::Hash(hash), None) => {
            let mut keys: Vec<String> = hash.borrow().keys().map(|k| k.to_string()).collect();
            keys.sort();
            keys
        }
        _ => return Err("Data must be an array of hashes".to_string()),
    };

    let rows: Vec<Vec<String>> = data_ref
        .iter()
        .map(|row| match row {
            Value::Hash(hash) => headers
                .iter()
                .map(|k| {
                    let key = crate::interpreter::value::HashKey::String(k.clone().into());
                    hash.borrow()
                        .get(&key)
                        .map(value_to_string)
                        .unwrap_or_default()
                })
                .collect(),
            _ => Vec::new(),
        })
        .collect();

    Ok((headers, rows))
}

fn to_csv_string(data: &Rc<RefCell<Vec<Value>>>) -> Result<String, String> {
    let (headers, rows) = extract_headers_and_rows(data)?;

    let line = |cells: &[String]| {
        cells
            .iter()
            .map(|cell| csv_field(cell))
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut csv_content = String::new();
    csv_content.push_str(&line(&headers));
    csv_content.push('\n');

    for row in rows {
        csv_content.push_str(&line(&row));
        csv_content.push('\n');
    }

    Ok(csv_content)
}

/// One cell of a CSV line.
///
/// Cells used to be joined as they were. A name holding a comma shifted its
/// row's columns, one holding a line break forged a row (`x\nroot,admin` in
/// an export an administrator opens), and a quote broke the file for other
/// parsers. A cell with any of those is quoted, its quotes doubled.
///
/// A cell that starts as a formula (`=`, `+`, `-`, `@`, a tab or a carriage
/// return) is what a spreadsheet runs when the file is opened: it gets a
/// leading apostrophe, which makes it text. Plain numbers are left alone.
fn csv_field(cell: &str) -> String {
    let formula =
        cell.starts_with(['=', '+', '-', '@', '\t', '\r']) && cell.parse::<f64>().is_err();
    let cell = if formula {
        std::borrow::Cow::Owned(format!("'{cell}"))
    } else {
        std::borrow::Cow::Borrowed(cell)
    };
    if cell.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.into_owned()
    }
}

fn write_csv_file(data: &Rc<RefCell<Vec<Value>>>, path: &str) -> Result<Value, String> {
    let csv_content = to_csv_string(data)?;
    // The jail and `O_NOFOLLOW`, as `File.write`: `File::create` followed a
    // symlink planted at the target and truncated whatever it pointed to.
    crate::interpreter::builtins::file::write_bytes_jailed(
        path,
        "Spreadsheet.csv_write",
        csv_content.as_bytes(),
    )?;
    Ok(Value::Null)
}

/// Spreadsheet column name for a 0-based index: 0 -> A, 25 -> Z, 26 -> AA.
fn column_letters(index: usize) -> String {
    let mut n = index + 1;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push((b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    out.iter().rev().collect()
}

/// One sheet to write: its name, rows and optional column list.
struct SheetSpec {
    name: String,
    data: Rc<RefCell<Vec<Value>>>,
    columns: Option<Vec<String>>,
}

fn write_excel_file(
    data: &Rc<RefCell<Vec<Value>>>,
    path: &str,
    columns: Option<Vec<String>>,
) -> Result<Value, String> {
    write_excel_workbook(
        vec![SheetSpec {
            name: "Sheet1".to_string(),
            data: data.clone(),
            columns,
        }],
        path,
    )
}

fn write_excel_workbook(sheets: Vec<SheetSpec>, path: &str) -> Result<Value, String> {
    if sheets.is_empty() {
        return Err("Spreadsheet.excel_write() needs at least one sheet".to_string());
    }
    let mut seen: Vec<String> = Vec::new();
    let mut workbook = XlsxWorkbook::default();
    for spec in sheets {
        check_sheet_name(&spec.name)?;
        let folded = spec.name.to_lowercase();
        if seen.contains(&folded) {
            return Err(format!(
                "Spreadsheet.excel_write() sheet name \"{}\" is used twice (Excel ignores case)",
                spec.name
            ));
        }
        seen.push(folded);
        workbook
            .sheets
            .push((spec.name, sheet_rows(&spec.data, spec.columns)?));
    }

    let bytes = workbook
        .to_bytes()
        .map_err(|e| format!("Spreadsheet.excel_write() cannot build {}: {}", path, e))?;
    // Built in memory, then written through the jail and `O_NOFOLLOW` like
    // `File.write`. `File::create(path)` took the string as given: a request
    // choosing the path could truncate any file the process could write.
    crate::interpreter::builtins::file::write_bytes_jailed(
        path,
        "Spreadsheet.excel_write",
        &bytes,
    )?;

    Ok(Value::Null)
}

/// Excel refuses to open a workbook whose sheet name is empty, longer than
/// 31 characters, holds one of `: \ / ? * [ ]`, starts or ends with an
/// apostrophe, or is the reserved `History`.
fn check_sheet_name(name: &str) -> Result<(), String> {
    let problem = if name.is_empty() {
        Some("is empty".to_string())
    } else if name.chars().count() > 31 {
        Some("is longer than 31 characters".to_string())
    } else if let Some(c) = name
        .chars()
        .find(|c| matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']'))
    {
        Some(format!("contains '{}'", c))
    } else if name.starts_with('\'') || name.ends_with('\'') {
        Some("starts or ends with an apostrophe".to_string())
    } else if name.eq_ignore_ascii_case("history") {
        Some("is reserved by Excel".to_string())
    } else {
        None
    };
    match problem {
        Some(why) => Err(format!(
            "Spreadsheet.excel_write() sheet name \"{}\" {}",
            name, why
        )),
        None => Ok(()),
    }
}

/// The `<sheetData>` rows of one sheet: a header row, then one row per hash.
fn sheet_rows(
    data: &Rc<RefCell<Vec<Value>>>,
    columns: Option<Vec<String>>,
) -> Result<String, String> {
    let (headers, rows) = extract_rows_for(data, columns)?;

    // Int and Float values are written as numbers (Excel can sum them);
    // everything else as text, Strings included even when they look numeric
    // (a postcode or an order number keeps its leading zeros).
    let data_ref = data.borrow();
    let mut sheet = XlsxSheet::new();
    sheet.row(headers.iter().map(|header| Cell::Text(header)));
    for (row_idx, row) in rows.iter().enumerate() {
        let source = match data_ref.get(row_idx) {
            Some(Value::Hash(hash)) => Some(hash.clone()),
            _ => None,
        };
        let cells: Vec<Cell> = row
            .iter()
            .enumerate()
            .map(|(col_idx, value)| {
                let original = source.as_ref().and_then(|hash| {
                    let key =
                        crate::interpreter::value::HashKey::String(headers[col_idx].clone().into());
                    hash.borrow().get(&key).cloned()
                });
                match original {
                    Some(Value::Int(n)) => Cell::Number(n as f64),
                    Some(Value::Float(f)) => Cell::Number(f),
                    _ => Cell::Text(value),
                }
            })
            .collect();
        sheet.row(cells);
    }
    Ok(sheet.rows)
}

/// One cell of a written sheet.
enum Cell<'a> {
    Number(f64),
    Text(&'a str),
}

/// The rows of one sheet, as `<row>` XML.
struct XlsxSheet {
    rows: String,
    row_count: usize,
}

impl XlsxSheet {
    fn new() -> Self {
        XlsxSheet {
            rows: String::new(),
            row_count: 0,
        }
    }

    fn row<'a>(&mut self, cells: impl IntoIterator<Item = Cell<'a>>) {
        use std::fmt::Write as _;
        self.row_count += 1;
        let row = self.row_count;
        let _ = write!(self.rows, "<row r=\"{row}\">");
        for (col_idx, cell) in cells.into_iter().enumerate() {
            let reference = format!("{}{row}", column_letters(col_idx));
            match cell {
                Cell::Number(n) if n.is_finite() => {
                    let _ = write!(self.rows, "<c r=\"{reference}\"><v>{n}</v></c>");
                }
                // NaN and infinities have no spreadsheet number; keep the text.
                Cell::Number(n) => {
                    let _ = write!(
                        self.rows,
                        "<c r=\"{reference}\" t=\"inlineStr\"><is><t>{n}</t></is></c>"
                    );
                }
                Cell::Text(text) => {
                    let _ = write!(
                        self.rows,
                        "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">"
                    );
                    push_xml_text(&mut self.rows, text);
                    self.rows.push_str("</t></is></c>");
                }
            }
        }
        self.rows.push_str("</row>");
    }
}

/// A workbook written as the smallest package Excel, LibreOffice, Numbers
/// and calamine all open: content types, two relationship parts, the
/// workbook, styles, and one part per sheet, in a zip. Text is stored inline
/// in its cell (`inlineStr`), which needs no shared-string table.
///
/// This replaced `umya-spreadsheet`, a full read-modify-write model of the
/// format (a megabyte of code, a dozen crates) used here for exactly this.
#[derive(Default)]
struct XlsxWorkbook {
    /// (sheet name, `<row>` XML), in tab order.
    sheets: Vec<(String, String)>,
}

impl XlsxWorkbook {
    /// The `.xlsx` package, as bytes.
    fn to_bytes(&self) -> Result<Vec<u8>, String> {
        use std::fmt::Write as _;
        let mut content_types = String::from(CONTENT_TYPES_HEAD);
        let mut workbook = String::from(WORKBOOK_HEAD);
        let mut workbook_rels = String::from(RELS_HEAD);
        for (idx, (name, _)) in self.sheets.iter().enumerate() {
            let n = idx + 1;
            let _ = write!(
                content_types,
                "<Override PartName=\"/xl/worksheets/sheet{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
            );
            workbook.push_str("<sheet name=\"");
            push_xml_attr(&mut workbook, name);
            let _ = write!(workbook, "\" sheetId=\"{n}\" r:id=\"rId{n}\"/>");
            let _ = write!(
                workbook_rels,
                "<Relationship Id=\"rId{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>"
            );
        }
        content_types.push_str("</Types>");
        workbook.push_str("</sheets></workbook>");
        let styles_id = self.sheets.len() + 1;
        let _ = write!(
            workbook_rels,
            "<Relationship Id=\"rId{styles_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>"
        );

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut parts: Vec<(String, String)> = vec![
            ("[Content_Types].xml".to_string(), content_types),
            ("_rels/.rels".to_string(), ROOT_RELS.to_string()),
            ("xl/workbook.xml".to_string(), workbook),
            ("xl/_rels/workbook.xml.rels".to_string(), workbook_rels),
            ("xl/styles.xml".to_string(), STYLES.to_string()),
        ];
        for (idx, (_, rows)) in self.sheets.iter().enumerate() {
            parts.push((
                format!("xl/worksheets/sheet{}.xml", idx + 1),
                format!(
                    "{XML_DECLARATION}<worksheet xmlns=\"{MAIN_NS}\"><sheetData>{rows}</sheetData></worksheet>"
                ),
            ));
        }
        for (name, content) in parts {
            zip.start_file(name, options).map_err(|e| e.to_string())?;
            zip.write_all(content.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
    }
}

/// Escape `text` for a double-quoted XML attribute.
fn push_xml_attr(out: &mut String, text: &str) {
    let mut escaped = String::new();
    push_xml_text(&mut escaped, text);
    out.push_str(&escaped.replace('"', "&quot;"));
}

/// Escape `text` for an XML text node. Control characters other than tab,
/// newline and carriage return cannot appear in XML at all and are dropped.
fn push_xml_text(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
}

const XML_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

const CONTENT_TYPES_HEAD: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
    "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    "<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
    "<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>",
);

const ROOT_RELS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>",
    "</Relationships>"
);

const WORKBOOK_HEAD: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ",
    "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
    "<sheets>",
);

const RELS_HEAD: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
);

/// The minimum Excel insists on: one font, the two reserved fills, one
/// border, one cell format.
const STYLES: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
    "<fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>",
    "<fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill>",
    "<fill><patternFill patternType=\"gray125\"/></fill></fills>",
    "<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>",
    "<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>",
    "<cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs>",
    "<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>",
    "</styleSheet>"
);

fn column_names(cols: &Rc<RefCell<Vec<Value>>>) -> Result<Vec<String>, String> {
    cols.borrow()
        .iter()
        .map(|c| match c {
            Value::String(s) => Ok(s.to_string()),
            other => Err(format!(
                "Spreadsheet.excel_write() columns must be strings, got {}",
                other.type_name()
            )),
        })
        .collect()
}

/// One entry of the hash of sheets: `rows` alone, or
/// `{"rows": rows, "columns": [...]}`.
fn sheet_spec(name: String, value: &Value) -> Result<SheetSpec, String> {
    match value {
        Value::Array(rows) => Ok(SheetSpec {
            name,
            data: rows.clone(),
            columns: None,
        }),
        Value::Hash(options) => {
            let options = options.borrow();
            let lookup = |key: &str| {
                options
                    .get(&crate::interpreter::value::HashKey::String(key.into()))
                    .cloned()
            };
            if let Some(unknown) = options
                .keys()
                .map(|k| k.to_string())
                .find(|k| k != "rows" && k != "columns")
            {
                return Err(format!(
                    "Spreadsheet.excel_write() sheet \"{}\" has an unknown option \"{}\" (expected rows, columns)",
                    name, unknown
                ));
            }
            let data = match lookup("rows") {
                Some(Value::Array(rows)) => rows,
                Some(other) => {
                    return Err(format!(
                        "Spreadsheet.excel_write() sheet \"{}\" rows must be an array, got {}",
                        name,
                        other.type_name()
                    ))
                }
                None => {
                    return Err(format!(
                        "Spreadsheet.excel_write() sheet \"{}\" needs \"rows\"",
                        name
                    ))
                }
            };
            let columns = match lookup("columns") {
                None | Some(Value::Null) => None,
                Some(Value::Array(cols)) => Some(column_names(&cols)?),
                Some(other) => {
                    return Err(format!(
                        "Spreadsheet.excel_write() sheet \"{}\" columns must be an array, got {}",
                        name,
                        other.type_name()
                    ))
                }
            };
            Ok(SheetSpec {
                name,
                data,
                columns,
            })
        }
        other => Err(format!(
            "Spreadsheet.excel_write() sheet \"{}\" expects an array of rows, got {}",
            name,
            other.type_name()
        )),
    }
}

fn get_spreadsheet_class() -> Rc<Class> {
    thread_local! {
        static CLASS: Rc<Class> = build_spreadsheet_class();
    }
    CLASS.with(|c| c.clone())
}

fn build_spreadsheet_class() -> Rc<Class> {
    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    static_methods.insert(
        "csv".to_string(),
        Rc::new(NativeFunction::new("Spreadsheet.csv", Some(1), |args| {
            let content = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(format!(
                        "Spreadsheet.csv() expects string, got {}",
                        args[0].type_name()
                    ))
                }
            };
            parse_csv_content(&content)
        })),
    );

    static_methods.insert(
        "csv_file".to_string(),
        Rc::new(NativeFunction::new(
            "Spreadsheet.csv_file",
            Some(1),
            |args| {
                let path = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(format!(
                            "Spreadsheet.csv_file() expects string path, got {}",
                            args[0].type_name()
                        ))
                    }
                };
                parse_csv_file(&path)
            },
        )),
    );

    static_methods.insert(
        "excel".to_string(),
        Rc::new(NativeFunction::new("Spreadsheet.excel", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "Spreadsheet.excel() expects 1-2 arguments (path, sheet?), got {}",
                    args.len()
                ));
            }
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(format!(
                        "Spreadsheet.excel() expects string path, got {}",
                        args[0].type_name()
                    ))
                }
            };
            let sheet = match args.get(1) {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.to_string()),
                Some(other) => {
                    let got = other.type_name();
                    return Err(format!(
                        "Spreadsheet.excel() expects a sheet name, got {}",
                        got
                    ));
                }
            };
            parse_excel_file(&path, sheet.as_deref())
        })),
    );

    static_methods.insert(
        "excel_sheets".to_string(),
        Rc::new(NativeFunction::new(
            "Spreadsheet.excel_sheets",
            Some(1),
            |args| {
                let path = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(format!(
                            "Spreadsheet.excel_sheets() expects string path, got {}",
                            args[0].type_name()
                        ))
                    }
                };
                parse_excel_sheets(&path)
            },
        )),
    );

    static_methods.insert(
        "to_csv".to_string(),
        Rc::new(NativeFunction::new("Spreadsheet.to_csv", Some(1), |args| {
            let data = match &args[0] {
                Value::Array(arr) => arr.clone(),
                _ => {
                    return Err(format!(
                        "Spreadsheet.to_csv() expects array, got {}",
                        args[0].type_name()
                    ))
                }
            };
            to_csv_string(&data).map(|s| Value::String(s.into()))
        })),
    );

    static_methods.insert(
        "csv_write".to_string(),
        Rc::new(NativeFunction::new(
            "Spreadsheet.csv_write",
            Some(2),
            |args| {
                let data = match &args[0] {
                    Value::Array(arr) => arr.clone(),
                    _ => {
                        return Err(format!(
                            "Spreadsheet.csv_write() expects array as first argument, got {}",
                            args[0].type_name()
                        ))
                    }
                };
                let path = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(format!(
                        "Spreadsheet.csv_write() expects string path as second argument, got {}",
                        args[1].type_name()
                    ))
                    }
                };
                write_csv_file(&data, &path)
            },
        )),
    );

    static_methods.insert(
        "excel_write".to_string(),
        Rc::new(NativeFunction::new(
            "Spreadsheet.excel_write",
            None,
            |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(format!(
                        "Spreadsheet.excel_write() expects 2-3 arguments (data, path, columns?), got {}",
                        args.len()
                    ));
                }
                let path = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(format!(
                        "Spreadsheet.excel_write() expects string path as second argument, got {}",
                        args[1].type_name()
                    ))
                    }
                };
                match &args[0] {
                    Value::Array(arr) => {
                        // Optional column list: order and selection of the keys.
                        let columns = match args.get(2) {
                            None | Some(Value::Null) => None,
                            Some(Value::Array(cols)) => Some(column_names(cols)?),
                            Some(other) => {
                                return Err(format!(
                                    "Spreadsheet.excel_write() expects an array of column names as third argument, got {}",
                                    other.type_name()
                                ))
                            }
                        };
                        write_excel_file(arr, &path, columns)
                    }
                    // Several sheets: name => rows, or name => {"rows", "columns"}.
                    Value::Hash(sheets) => {
                        if args.len() > 2 {
                            return Err("Spreadsheet.excel_write() with a hash of sheets takes no third argument; give each sheet its own {\"rows\": …, \"columns\": […]}".to_string());
                        }
                        let specs = sheets
                            .borrow()
                            .iter()
                            .map(|(name, value)| sheet_spec(name.to_string(), value))
                            .collect::<Result<Vec<SheetSpec>, String>>()?;
                        write_excel_workbook(specs, &path)
                    }
                    other => Err(format!(
                        "Spreadsheet.excel_write() expects an array of rows or a hash of sheets as first argument, got {}",
                        other.type_name()
                    )),
                }
            },
        )),
    );

    let spreadsheet_class = Class {
        name: "Spreadsheet".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    Rc::new(spreadsheet_class)
}

pub fn register_spreadsheet_class(env: &mut Environment) {
    let class = get_spreadsheet_class();
    env.define("Spreadsheet".to_string(), Value::Class(class));
}

pub fn register_spreadsheet_builtins(env: &mut Environment) {
    register_spreadsheet_class(env);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::hash_from_pairs;
    use calamine::{Data, Reader};

    fn row(pairs: &[(&str, Value)]) -> Value {
        hash_from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())))
    }

    #[test]
    fn column_letters_go_past_z() {
        assert_eq!(column_letters(0), "A");
        assert_eq!(column_letters(25), "Z");
        assert_eq!(column_letters(26), "AA");
        assert_eq!(column_letters(27), "AB");
        assert_eq!(column_letters(701), "ZZ");
        assert_eq!(column_letters(702), "AAA");
    }

    #[test]
    fn excel_write_keeps_the_given_column_order_and_number_types() {
        let data = Rc::new(RefCell::new(vec![
            row(&[
                ("Montant", Value::Float(25.5)),
                ("N°", Value::String("00042".into())),
                ("Places", Value::Int(2)),
            ]),
            row(&[
                ("Montant", Value::Float(10.0)),
                ("N°", Value::String("43".into())),
            ]),
        ]));
        let dir = std::env::temp_dir().join(format!("soli_xlsx_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("order.xlsx");
        let columns = vec![
            "N°".to_string(),
            "Places".to_string(),
            "Montant".to_string(),
        ];
        write_excel_file(&data, path.to_str().unwrap(), Some(columns)).unwrap();

        let mut book: calamine::Xlsx<_> = calamine::open_workbook(&path).unwrap();
        let name = book.sheet_names().first().cloned().unwrap();
        let range = book.worksheet_range(&name).unwrap();
        let header: Vec<String> = range
            .rows()
            .next()
            .unwrap()
            .iter()
            .map(|c| c.to_string())
            .collect();
        assert_eq!(header, vec!["N°", "Places", "Montant"]);
        let first: Vec<&Data> = range.rows().nth(1).unwrap().iter().collect();
        assert_eq!(first[0], &Data::String("00042".into()));
        assert!(matches!(first[1], Data::Float(f) if *f == 2.0));
        assert!(matches!(first[2], Data::Float(f) if *f == 25.5));
        // A key missing from a row leaves its cell empty.
        let second: Vec<&Data> = range.rows().nth(2).unwrap().iter().collect();
        assert!(matches!(second[1], Data::Empty) || second[1] == &Data::String(String::new()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn excel_write_writes_one_sheet_per_entry_in_order() {
        let dir = std::env::temp_dir().join(format!("soli_xlsx_multi_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.xlsx");
        let specs = vec![
            SheetSpec {
                name: "Clients & co".to_string(),
                data: Rc::new(RefCell::new(vec![row(&[(
                    "nom",
                    Value::String("Ada".into()),
                )])])),
                columns: None,
            },
            SheetSpec {
                name: "Commandes".to_string(),
                data: Rc::new(RefCell::new(vec![
                    row(&[("n", Value::Int(1)), ("total", Value::Float(9.5))]),
                    row(&[("n", Value::Int(2)), ("total", Value::Float(3.0))]),
                ])),
                columns: Some(vec!["total".to_string(), "n".to_string()]),
            },
        ];
        write_excel_workbook(specs, path.to_str().unwrap()).unwrap();

        let mut book: calamine::Xlsx<_> = calamine::open_workbook(&path).unwrap();
        assert_eq!(book.sheet_names(), vec!["Clients & co", "Commandes"]);
        let orders = book.worksheet_range("Commandes").unwrap();
        assert_eq!(orders.height(), 3);
        assert_eq!(orders.get((0, 0)), Some(&Data::String("total".into())));
        assert!(matches!(orders.get((1, 0)), Some(Data::Float(f)) if *f == 9.5));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sheet_names_excel_would_refuse_are_rejected() {
        for bad in ["", "a/b", "x[1]", "'quoted'", "History", &"x".repeat(32)] {
            assert!(check_sheet_name(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(check_sheet_name("Ventes 2026 — T1").is_ok());
        assert!(check_sheet_name(&"é".repeat(31)).is_ok());
    }

    #[test]
    fn a_csv_cell_cannot_shift_a_column_forge_a_row_or_run_a_formula() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("x,admin"), "\"x,admin\"");
        assert_eq!(csv_field("x\nroot,admin"), "\"x\nroot,admin\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(
            csv_field("=HYPERLINK(\"http://evil\")"),
            "\"'=HYPERLINK(\"\"http://evil\"\")\""
        );
        assert_eq!(csv_field("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv_field("+cmd|x"), "'+cmd|x");
        // Numbers stay numbers.
        assert_eq!(csv_field("-12.5"), "-12.5");
        assert_eq!(csv_field("+3"), "+3");
        assert_eq!(csv_field(""), "");
    }
}
