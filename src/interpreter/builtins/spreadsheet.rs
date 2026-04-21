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
                let key = crate::interpreter::value::HashKey::String(k.clone());
                let value = if v.is_empty() {
                    Value::Null
                } else {
                    Value::String(v.to_string())
                };
                (key, value)
            })
            .collect::<HashPairs>();
        rows.push(Value::Hash(Rc::new(RefCell::new(hash_pairs))));
    }

    Ok(Value::Array(Rc::new(RefCell::new(rows))))
}

fn parse_csv_file(path: &str) -> Result<Value, String> {
    let file = File::open(path)
        .map_err(|e| format!("Spreadsheet.csv_file() cannot open {}: {}", path, e))?;
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
                let key = crate::interpreter::value::HashKey::String(k.clone());
                let value = if v.is_empty() {
                    Value::Null
                } else {
                    Value::String(v.to_string())
                };
                (key, value)
            })
            .collect::<HashPairs>();
        rows.push(Value::Hash(Rc::new(RefCell::new(hash_pairs))));
    }

    Ok(Value::Array(Rc::new(RefCell::new(rows))))
}

fn parse_excel_file(path: &str) -> Result<Value, String> {
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
                let key = crate::interpreter::value::HashKey::String(k.clone());
                let value = match v {
                    calamine::Data::Empty => Value::Null,
                    calamine::Data::String(s) => Value::String(s.clone()),
                    calamine::Data::Float(f) => Value::Float(*f),
                    calamine::Data::Int(i) => Value::Int(*i),
                    calamine::Data::Bool(b) => Value::Bool(*b),
                    calamine::Data::DateTime(dt) => Value::String(dt.to_string()),
                    calamine::Data::Error(e) => Value::String(format!("<error: {:?}>", e)),
                    calamine::Data::DateTimeIso(s) => Value::String(s.clone()),
                    calamine::Data::DurationIso(s) => Value::String(s.clone()),
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
        Value::String(s) => s.clone(),
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
    let data_ref = data.borrow();
    if data_ref.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let first_row = data_ref.first().ok_or("Data array is empty")?;

    let headers = match first_row {
        Value::Hash(hash) => {
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
                    let key = crate::interpreter::value::HashKey::String(k.clone());
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
    let mut file = std::fs::File::create(path)
        .map_err(|e| format!("Spreadsheet.csv_write() cannot create {}: {}", path, e))?;
    std::io::Write::write_all(&mut file, csv_content.as_bytes())
        .map_err(|e| format!("Spreadsheet.csv_write() cannot write to {}: {}", path, e))?;
    Ok(Value::Null)
}

fn write_excel_file(data: &Rc<RefCell<Vec<Value>>>, path: &str) -> Result<Value, String> {
    let (headers, rows) = extract_headers_and_rows(data)?;

    let mut spreadsheet = new_file();
    let worksheet = spreadsheet.get_sheet_mut(&0).unwrap();

    for (col_idx, header) in headers.iter().enumerate() {
        let col_letter = (b'A' + col_idx as u8) as char;
        worksheet
            .get_cell_mut(format!("{col_letter}1"))
            .set_value(header.clone());
    }

    for (row_idx, row) in rows.iter().enumerate() {
        let row_number = row_idx + 2;
        for (col_idx, value) in row.iter().enumerate() {
            let col_letter = (b'A' + col_idx as u8) as char;
            worksheet
                .get_cell_mut(format!("{col_letter}{row_number}"))
                .set_value(value.clone());
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
            to_csv_string(&data).map(Value::String)
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
            Some(2),
            |args| {
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
                write_excel_file(&data, &path)
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
