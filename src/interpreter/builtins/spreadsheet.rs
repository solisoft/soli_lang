//! Spreadsheet parsing and export built-in functions (CSV and Excel).
//!
//! Provides the Spreadsheet class for parsing and exporting spreadsheet files:
//! - Spreadsheet.csv(content) - Parse CSV string to array
//! - Spreadsheet.csv_file(path) - Parse CSV file to array
//! - Spreadsheet.excel(path) - Parse Excel file to array
//! - Spreadsheet.to_csv(data) - Convert array to CSV string
//! - Spreadsheet.csv_write(data, path) - Write array to CSV file
//! - Spreadsheet.excel_write(data, path) - Write array to Excel file

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::rc::Rc;

use calamine::{open_workbook, Reader, Xlsx};
use csv::ReaderBuilder;
use umya_spreadsheet::{new_file, writer};

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

fn parse_excel_file(path: &str) -> Result<Value, String> {
    // SECURITY: calamine parses the ZIP-embedded XML with a transitive
    // quick-xml < 0.41, which carries the DoS advisories RUSTSEC-2026-0194 /
    // -0195 (quadratic attribute scan, unbounded namespace allocation). No
    // upstream release reaches quick-xml >= 0.41 yet, so treat `path` as
    // trusted input — do not parse attacker-supplied spreadsheets on a request
    // path without an out-of-band size/timeout sandbox. Tracked in
    // tasks/todo/transitive-quick-xml-dos-spreadsheets.md.
    let mut workbook: Xlsx<_> = open_workbook(path)
        .map_err(|e| format!("Spreadsheet.excel() cannot open {}: {}", path, e))?;

    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| "Spreadsheet.excel() file has no sheets".to_string())?;

    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|e| format!("Spreadsheet.excel() error reading sheet: {}", e))?;

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

    let mut csv_content = String::new();
    csv_content.push_str(&headers.join(","));
    csv_content.push('\n');

    for row in rows {
        csv_content.push_str(&row.join(","));
        csv_content.push('\n');
    }

    Ok(csv_content)
}

fn write_csv_file(data: &Rc<RefCell<Vec<Value>>>, path: &str) -> Result<Value, String> {
    let csv_content = to_csv_string(data)?;
    let path =
        &crate::interpreter::builtins::file::resolve_readable_path(path, "Spreadsheet.csv_write")?;
    let mut file = std::fs::File::create(path).map_err(|e| {
        format!(
            "Spreadsheet.csv_write() cannot create {}: {}",
            path.display(),
            e
        )
    })?;
    std::io::Write::write_all(&mut file, csv_content.as_bytes()).map_err(|e| {
        format!(
            "Spreadsheet.csv_write() cannot write to {}: {}",
            path.display(),
            e
        )
    })?;
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

fn write_excel_file(
    data: &Rc<RefCell<Vec<Value>>>,
    path: &str,
    columns: Option<Vec<String>>,
) -> Result<Value, String> {
    let (headers, rows) = extract_rows_for(data, columns)?;

    let mut spreadsheet = new_file();
    // umya 3.x takes the index by value and answers a `Result`; `new_file()`
    // always makes sheet 0, so a failure here would be the crate breaking its
    // own contract rather than anything the caller did — but it is reported
    // rather than unwrapped, because this runs on a request.
    let worksheet = spreadsheet
        .sheet_mut(0)
        .map_err(|e| format!("Spreadsheet.excel_write() cannot open the sheet: {e}"))?;

    for (col_idx, header) in headers.iter().enumerate() {
        let col_letter = column_letters(col_idx);
        worksheet
            .cell_mut(format!("{col_letter}1"))
            .set_value(header.clone());
    }

    // Int and Float values are written as numbers (Excel can sum them);
    // everything else as text, Strings included even when they look numeric
    // (a postcode or an order number keeps its leading zeros).
    let data_ref = data.borrow();
    for (row_idx, row) in rows.iter().enumerate() {
        let row_number = row_idx + 2;
        let source = match data_ref.get(row_idx) {
            Some(Value::Hash(hash)) => Some(hash.clone()),
            _ => None,
        };
        for (col_idx, value) in row.iter().enumerate() {
            let col_letter = column_letters(col_idx);
            let cell = worksheet.cell_mut(format!("{col_letter}{row_number}"));
            let original = source.as_ref().and_then(|hash| {
                let key =
                    crate::interpreter::value::HashKey::String(headers[col_idx].clone().into());
                hash.borrow().get(&key).cloned()
            });
            match original {
                Some(Value::Int(n)) => {
                    cell.set_value_number(n as f64);
                }
                Some(Value::Float(f)) => {
                    cell.set_value_number(f);
                }
                _ => {
                    cell.set_value_string(value.clone());
                }
            }
        }
    }

    let target = std::path::Path::new(path);
    writer::xlsx::write(&spreadsheet, target)
        .map_err(|e| format!("Spreadsheet.excel_write() cannot write to {}: {}", path, e))?;

    Ok(Value::Null)
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
        Rc::new(NativeFunction::new("Spreadsheet.excel", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => {
                    return Err(format!(
                        "Spreadsheet.excel() expects string path, got {}",
                        args[0].type_name()
                    ))
                }
            };
            parse_excel_file(&path)
        })),
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
                let data = match &args[0] {
                    Value::Array(arr) => arr.clone(),
                    _ => {
                        return Err(format!(
                            "Spreadsheet.excel_write() expects array as first argument, got {}",
                            args[0].type_name()
                        ))
                    }
                };
                let path = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(format!(
                        "Spreadsheet.excel_write() expects string path as second argument, got {}",
                        args[1].type_name()
                    ))
                    }
                };
                // Optional column list: order and selection of the keys.
                let columns = match args.get(2) {
                    None | Some(Value::Null) => None,
                    Some(Value::Array(cols)) => Some(
                        cols.borrow()
                            .iter()
                            .map(|c| match c {
                                Value::String(s) => Ok(s.to_string()),
                                other => Err(format!(
                                    "Spreadsheet.excel_write() columns must be strings, got {}",
                                    other.type_name()
                                )),
                            })
                            .collect::<Result<Vec<String>, String>>()?,
                    ),
                    Some(other) => {
                        return Err(format!(
                            "Spreadsheet.excel_write() expects an array of column names as third argument, got {}",
                            other.type_name()
                        ))
                    }
                };
                write_excel_file(&data, &path, columns)
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
}
