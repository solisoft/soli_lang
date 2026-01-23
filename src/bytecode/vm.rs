//! Stack-based virtual machine for executing bytecode.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use chrono::{Datelike, Timelike};

// Thread-local storage for I18n locale
thread_local! {
    static VM_CURRENT_LOCALE: RefCell<String> = RefCell::new("en".to_string());
}

fn vm_get_locale() -> String {
    VM_CURRENT_LOCALE.with(|l| l.borrow().clone())
}

fn vm_set_locale(locale: String) {
    VM_CURRENT_LOCALE.with(|l| *l.borrow_mut() = locale);
}

use crate::bytecode::chunk::{
    Chunk, Closure, CompiledFunction, Constant, Upvalue, VMClass, VMInstance, VMIterator, VMValue,
};
use crate::bytecode::instruction::OpCode;
use crate::error::RuntimeError;
use crate::span::Span;

/// Maximum stack size.
const STACK_MAX: usize = 65536;
/// Maximum call frames.
const FRAMES_MAX: usize = 256;

/// Parse a datetime string into a Unix timestamp.
fn parse_datetime_string(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let datetime = if s.ends_with('Z') || s.contains("+") {
        chrono::DateTime::parse_from_rfc3339(s).or_else(|_| chrono::DateTime::parse_from_rfc2822(s))
    } else {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
            .map(|nd| {
                chrono::DateTime::from_naive_utc_and_offset(
                    nd,
                    chrono::FixedOffset::east_opt(0).unwrap(),
                )
            })
            .or_else(|_| {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map(|d| {
                    chrono::DateTime::from_naive_utc_and_offset(
                        d.and_hms_opt(0, 0, 0).unwrap(),
                        chrono::FixedOffset::east_opt(0).unwrap(),
                    )
                })
            })
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").map(|nd| {
                    chrono::DateTime::from_naive_utc_and_offset(
                        nd,
                        chrono::FixedOffset::east_opt(0).unwrap(),
                    )
                })
            })
    };
    match datetime {
        Ok(dt) => Ok(dt.timestamp()),
        Err(_) => Err(format!("Invalid datetime format: {}", s)),
    }
}

/// Result type for VM operations.
pub type VMResult<T> = Result<T, RuntimeError>;

/// A call frame representing a function invocation.
#[derive(Debug, Clone)]
struct CallFrame {
    /// The closure being executed
    closure: Rc<RefCell<Closure>>,
    /// Instruction pointer (offset into chunk.code)
    ip: usize,
    /// Base pointer into the stack (where this frame's slots start)
    slots_start: usize,
}

/// Native function identifiers.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
enum NativeId {
    Print = 0,
    Println,
    Input,
    Len,
    Push,
    Pop,
    Shift,
    Unshift,
    Slice,
    ToString,
    Str,
    ToInt,
    ToFloat,
    Upcase,
    Downcase,
    Trim,
    Split,
    Join,
    Contains,
    IndexOf,
    Substring,
    Map,
    Filter,
    Fold,
    Reverse,
    Sort,
    TypeOf,
    IsNull,
    Now,
    Clock,
    Range,
    Abs,
    Min,
    Max,
    Floor,
    Ceil,
    Round,
    Sqrt,
    Pow,
    Keys,
    Values,
    Entries,
    FromEntries,
    HasKey,
    Delete,
    Merge,
    Clear,
    // HTTP functions
    HttpGet,
    HttpPost,
    HttpGetJson,
    HttpPostJson,
    HttpRequest,
    JsonParse,
    JsonStringify,
    HttpOk,
    HttpSuccess,
    HttpRedirect,
    HttpClientError,
    HttpServerError,
    // File I/O functions
    Barf,
    Slurp,
    // HTML functions
    HtmlEscape,
    HtmlUnescape,
    SanitizeHtml,
    StripHtml,
    // Regex functions
    RegexMatch,
    RegexFind,
    RegexFindAll,
    RegexReplace,
    RegexReplaceAll,
    RegexSplit,
    RegexCapture,
    RegexEscape,
    // DateTime functions
    DateTimeNow,
    DateTimeParse,
    DateTimeUtc,
    DurationBetween,
    DurationSeconds,
    DurationMinutes,
    DurationHours,
    DurationDays,
    DurationWeeks,
    // Async functions
    Await,
}

impl NativeId {
    fn from_u16(val: u16) -> Option<Self> {
        if val <= NativeId::Await as u16 {
            Some(unsafe { std::mem::transmute(val) })
        } else {
            None
        }
    }

    fn arity(self) -> Option<usize> {
        match self {
            NativeId::Print => None,   // variadic
            NativeId::Println => None, // variadic
            NativeId::Input => Some(0),
            NativeId::Len => Some(1),
            NativeId::Push => Some(2),
            NativeId::Pop => Some(1),
            NativeId::Shift => Some(1),
            NativeId::Unshift => Some(2),
            NativeId::Slice => Some(3),
            NativeId::ToString => Some(1),
            NativeId::Str => Some(1),
            NativeId::ToInt => Some(1),
            NativeId::ToFloat => Some(1),
            NativeId::Upcase => Some(1),
            NativeId::Downcase => Some(1),
            NativeId::Trim => Some(1),
            NativeId::Split => Some(2),
            NativeId::Join => Some(2),
            NativeId::Contains => Some(2),
            NativeId::IndexOf => Some(2),
            NativeId::Substring => Some(3),
            NativeId::Map => Some(2),
            NativeId::Filter => Some(2),
            NativeId::Fold => Some(3),
            NativeId::Reverse => Some(1),
            NativeId::Sort => Some(1),
            NativeId::TypeOf => Some(1),
            NativeId::IsNull => Some(1),
            NativeId::Now => Some(0),
            NativeId::Clock => Some(0),
            NativeId::Range => Some(2),
            NativeId::Abs => Some(1),
            NativeId::Min => Some(2),
            NativeId::Max => Some(2),
            NativeId::Floor => Some(1),
            NativeId::Ceil => Some(1),
            NativeId::Round => Some(1),
            NativeId::Sqrt => Some(1),
            NativeId::Pow => Some(2),
            NativeId::Keys => Some(1),
            NativeId::Values => Some(1),
            NativeId::Entries => Some(1),
            NativeId::FromEntries => Some(1),
            NativeId::HasKey => Some(2),
            NativeId::Delete => Some(2),
            NativeId::Merge => Some(2),
            NativeId::Clear => Some(1),
            // HTTP functions
            NativeId::HttpGet => Some(1),
            NativeId::HttpPost => Some(2),
            NativeId::HttpGetJson => Some(1),
            NativeId::HttpPostJson => Some(2),
            NativeId::HttpRequest => None, // variadic (2-4 args)
            NativeId::JsonParse => Some(1),
            NativeId::JsonStringify => Some(1),
            NativeId::HttpOk => Some(1),
            NativeId::HttpSuccess => Some(1),
            NativeId::HttpRedirect => Some(1),
            NativeId::HttpClientError => Some(1),
            NativeId::HttpServerError => Some(1),
            // File I/O functions
            NativeId::Barf => None,  // variadic: 2-3 args
            NativeId::Slurp => None, // variadic: 1-2 args
            // HTML functions
            NativeId::HtmlEscape => Some(1),
            NativeId::HtmlUnescape => Some(1),
            NativeId::SanitizeHtml => Some(1),
            NativeId::StripHtml => Some(1),
            // Regex functions
            NativeId::RegexMatch => Some(2),
            NativeId::RegexFind => Some(2),
            NativeId::RegexFindAll => Some(2),
            NativeId::RegexReplace => Some(3),
            NativeId::RegexReplaceAll => Some(3),
            NativeId::RegexSplit => Some(2),
            NativeId::RegexCapture => Some(2),
            NativeId::RegexEscape => Some(1),
            // DateTime functions
            NativeId::DateTimeNow => Some(0),
            NativeId::DateTimeParse => Some(1),
            NativeId::DateTimeUtc => Some(0),
            NativeId::DurationBetween => Some(2),
            NativeId::DurationSeconds => Some(1),
            NativeId::DurationMinutes => Some(1),
            NativeId::DurationHours => Some(1),
            NativeId::DurationDays => Some(1),
            NativeId::DurationWeeks => Some(1),
            // Async functions
            NativeId::Await => Some(1),
        }
    }
}

/// Call a native method on a DateTime or Duration instance.
fn call_datetime_method(inst: &VMInstance, method_name: &str) -> Result<VMValue, String> {
    let timestamp = inst
        .get("timestamp")
        .or_else(|| inst.get("seconds"))
        .and_then(|v| match v {
            VMValue::Int(t) => Some(t),
            VMValue::Float(t) => Some(t as i64),
            _ => None,
        })
        .ok_or_else(|| format!("DateTime/Duration instance missing timestamp field"))?;

    // Use chrono to parse the timestamp
    let datetime = chrono::DateTime::from_timestamp(timestamp, 0)
        .ok_or_else(|| format!("Invalid timestamp: {}", timestamp))?;

    match method_name {
        "year" => Ok(VMValue::Int(datetime.year() as i64)),
        "month" => Ok(VMValue::Int(datetime.month() as i64)),
        "day" => Ok(VMValue::Int(datetime.day() as i64)),
        "hour" => Ok(VMValue::Int(datetime.hour() as i64)),
        "minute" => Ok(VMValue::Int(datetime.minute() as i64)),
        "second" => Ok(VMValue::Int(datetime.second() as i64)),
        "weekday" => Ok(VMValue::String(Rc::new(match datetime.weekday() {
            chrono::Weekday::Mon => "monday".to_string(),
            chrono::Weekday::Tue => "tuesday".to_string(),
            chrono::Weekday::Wed => "wednesday".to_string(),
            chrono::Weekday::Thu => "thursday".to_string(),
            chrono::Weekday::Fri => "friday".to_string(),
            chrono::Weekday::Sat => "saturday".to_string(),
            chrono::Weekday::Sun => "sunday".to_string(),
        }))),
        "to_unix" => Ok(VMValue::Int(timestamp)),
        "to_iso" => {
            let iso = datetime.to_rfc3339();
            Ok(VMValue::String(Rc::new(iso)))
        }
        "to_string" | "str" => {
            let s = format!("{}", datetime);
            Ok(VMValue::String(Rc::new(s)))
        }
        "total_seconds" => {
            let seconds = inst
                .get("seconds")
                .or_else(|| inst.get("timestamp"))
                .and_then(|v| match v {
                    VMValue::Int(t) => Some(t as f64),
                    VMValue::Float(t) => Some(t),
                    _ => None,
                })
                .unwrap_or(0.0);
            Ok(VMValue::Float(seconds))
        }
        "total_minutes" => {
            let seconds = inst
                .get("seconds")
                .or_else(|| inst.get("timestamp"))
                .and_then(|v| match v {
                    VMValue::Int(t) => Some(t as f64),
                    VMValue::Float(t) => Some(t),
                    _ => None,
                })
                .unwrap_or(0.0);
            Ok(VMValue::Float(seconds / 60.0))
        }
        "total_hours" => {
            let seconds = inst
                .get("seconds")
                .or_else(|| inst.get("timestamp"))
                .and_then(|v| match v {
                    VMValue::Int(t) => Some(t as f64),
                    VMValue::Float(t) => Some(t),
                    _ => None,
                })
                .unwrap_or(0.0);
            Ok(VMValue::Float(seconds / 3600.0))
        }
        "total_days" => {
            let seconds = inst
                .get("seconds")
                .or_else(|| inst.get("timestamp"))
                .and_then(|v| match v {
                    VMValue::Int(t) => Some(t as f64),
                    VMValue::Float(t) => Some(t),
                    _ => None,
                })
                .unwrap_or(0.0);
            Ok(VMValue::Float(seconds / 86400.0))
        }
        "total_weeks" => {
            let seconds = inst
                .get("seconds")
                .or_else(|| inst.get("timestamp"))
                .and_then(|v| match v {
                    VMValue::Int(t) => Some(t as f64),
                    VMValue::Float(t) => Some(t),
                    _ => None,
                })
                .unwrap_or(0.0);
            Ok(VMValue::Float(seconds / 604800.0))
        }
        _ => Err(format!("Unknown DateTime/Duration method: {}", method_name)),
    }
}

/// Helper to create a CompiledFunction for datetime_utc
fn create_datetime_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "datetime_utc".to_string(),
        arity: 0,
        full_arity: 0,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a CompiledFunction for datetime_parse
fn create_datetime_parse_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "datetime_parse".to_string(),
        arity: 1,
        full_arity: 1,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a CompiledFunction for duration_between
fn create_duration_between_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "duration_between".to_string(),
        arity: 2,
        full_arity: 2,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a CompiledFunction for duration constructors
fn create_duration_from_value_closure(method_name: String) -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: format!("duration_{}", method_name),
        arity: 1,
        full_arity: 1,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a closure for I18n.locale()
fn create_i18n_locale_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "i18n_locale".to_string(),
        arity: 0,
        full_arity: 0,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a closure for I18n.set_locale(locale)
fn create_i18n_set_locale_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "i18n_set_locale".to_string(),
        arity: 1,
        full_arity: 1,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a closure for I18n.translate(key, locale?, translations?)
fn create_i18n_translate_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "i18n_translate".to_string(),
        arity: 1,
        full_arity: 3,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a closure for I18n.plural(key, n, locale?, translations?)
fn create_i18n_plural_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "i18n_plural".to_string(),
        arity: 2,
        full_arity: 4,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a closure for I18n.format_number(n, locale?)
fn create_i18n_format_number_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "i18n_format_number".to_string(),
        arity: 1,
        full_arity: 2,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a closure for I18n.format_currency(amount, currency, locale?)
fn create_i18n_format_currency_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "i18n_format_currency".to_string(),
        arity: 2,
        full_arity: 3,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// Helper to create a closure for I18n.format_date(ts, locale?)
fn create_i18n_format_date_closure() -> Rc<RefCell<Closure>> {
    let function = Rc::new(CompiledFunction {
        name: "i18n_format_date".to_string(),
        arity: 1,
        full_arity: 2,
        chunk: Chunk::new(),
        upvalue_count: 0,
        is_method: false,
        default_values: Vec::new(),
    });
    Rc::new(RefCell::new(Closure::new(function)))
}

/// The bytecode virtual machine.
pub struct VM {
    /// The value stack
    stack: Vec<VMValue>,
    /// Call frames
    frames: Vec<CallFrame>,
    /// Global variables
    globals: HashMap<String, VMValue>,
    /// Open upvalues (stack slot -> upvalue)
    open_upvalues: Vec<Rc<RefCell<Upvalue>>>,
    /// Exception handler stack
    exception_handlers: Vec<ExceptionHandler>,
    /// Current exception being thrown (if any)
    current_exception: Option<VMValue>,
}

/// An exception handler on the stack.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ExceptionHandler {
    /// IP to jump to for catch block (0 if no catch)
    catch_ip: usize,
    /// IP to jump to for finally block (0 if no finally)
    finally_ip: usize,
    /// Stack depth at handler entry (for unwinding)
    stack_depth: usize,
    /// Number of local slots to pop on entry
    local_slots: usize,
}

impl VM {
    /// Create a new VM.
    pub fn new() -> Self {
        let mut vm = Self {
            stack: Vec::with_capacity(STACK_MAX),
            frames: Vec::with_capacity(FRAMES_MAX),
            globals: HashMap::new(),
            open_upvalues: Vec::new(),
            exception_handlers: Vec::new(),
            current_exception: None,
        };

        // Register DateTime and Duration classes
        let datetime_class = Rc::new(RefCell::new(VMClass::new("DateTime".to_string())));
        vm.globals.insert(
            "DateTime".to_string(),
            VMValue::Class(datetime_class.clone()),
        );

        let duration_class = Rc::new(RefCell::new(VMClass::new("Duration".to_string())));
        vm.globals.insert(
            "Duration".to_string(),
            VMValue::Class(duration_class.clone()),
        );

        // Register I18n class
        let i18n_class = Rc::new(RefCell::new(VMClass::new("I18n".to_string())));
        vm.globals
            .insert("I18n".to_string(), VMValue::Class(i18n_class.clone()));

        vm
    }

    /// Call a native function by ID.
    fn call_native(&mut self, id: NativeId, args: Vec<VMValue>) -> VMResult<VMValue> {
        match id {
            NativeId::Print => {
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        print!(" ");
                    }
                    print!("{}", arg);
                }
                println!();
                Ok(VMValue::Null)
            }
            NativeId::Println => {
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        print!(" ");
                    }
                    print!("{}", arg);
                }
                println!();
                Ok(VMValue::Null)
            }
            NativeId::Input => {
                let mut input = String::new();
                io::stdin().read_line(&mut input).map_err(|e| {
                    RuntimeError::new(format!("Failed to read input: {}", e), Span::default())
                })?;
                Ok(VMValue::String(Rc::new(input.trim_end().to_string())))
            }
            NativeId::Len => match &args[0] {
                VMValue::String(s) => Ok(VMValue::Int(s.len() as i64)),
                VMValue::Array(arr) => Ok(VMValue::Int(arr.borrow().len() as i64)),
                VMValue::Hash(hash) => Ok(VMValue::Int(hash.borrow().len() as i64)),
                other => Err(RuntimeError::new(
                    format!("Cannot get length of {}", other.type_name()),
                    Span::default(),
                )),
            },
            NativeId::Push => {
                if let VMValue::Array(arr) = &args[0] {
                    arr.borrow_mut().push(args[1].clone());
                    Ok(VMValue::Null)
                } else {
                    Err(RuntimeError::new("push requires an array", Span::default()))
                }
            }
            NativeId::Pop => {
                if let VMValue::Array(arr) = &args[0] {
                    arr.borrow_mut().pop().ok_or_else(|| {
                        RuntimeError::new("Cannot pop from empty array", Span::default())
                    })
                } else {
                    Err(RuntimeError::new("pop requires an array", Span::default()))
                }
            }
            NativeId::Shift => {
                if let VMValue::Array(arr) = &args[0] {
                    let mut arr = arr.borrow_mut();
                    if arr.is_empty() {
                        Err(RuntimeError::new(
                            "Cannot shift from empty array",
                            Span::default(),
                        ))
                    } else {
                        Ok(arr.remove(0))
                    }
                } else {
                    Err(RuntimeError::new(
                        "shift requires an array",
                        Span::default(),
                    ))
                }
            }
            NativeId::Unshift => {
                if let VMValue::Array(arr) = &args[0] {
                    arr.borrow_mut().insert(0, args[1].clone());
                    Ok(VMValue::Null)
                } else {
                    Err(RuntimeError::new(
                        "unshift requires an array",
                        Span::default(),
                    ))
                }
            }
            NativeId::Slice => match &args[0] {
                VMValue::Array(arr) => {
                    let arr = arr.borrow();
                    let start = match &args[1] {
                        VMValue::Int(n) => *n as usize,
                        _ => {
                            return Err(RuntimeError::new(
                                "slice start must be Int",
                                Span::default(),
                            ))
                        }
                    };
                    let end = match &args[2] {
                        VMValue::Int(n) => *n as usize,
                        _ => {
                            return Err(RuntimeError::new("slice end must be Int", Span::default()))
                        }
                    };
                    let end = end.min(arr.len());
                    let start = start.min(end);
                    let sliced: Vec<VMValue> = arr[start..end].to_vec();
                    Ok(VMValue::Array(Rc::new(RefCell::new(sliced))))
                }
                VMValue::String(s) => {
                    let start = match &args[1] {
                        VMValue::Int(n) => *n as usize,
                        _ => {
                            return Err(RuntimeError::new(
                                "slice start must be Int",
                                Span::default(),
                            ))
                        }
                    };
                    let end = match &args[2] {
                        VMValue::Int(n) => *n as usize,
                        _ => {
                            return Err(RuntimeError::new("slice end must be Int", Span::default()))
                        }
                    };
                    let chars: Vec<char> = s.chars().collect();
                    let end = end.min(chars.len());
                    let start = start.min(end);
                    let sliced: String = chars[start..end].iter().collect();
                    Ok(VMValue::String(Rc::new(sliced)))
                }
                _ => Err(RuntimeError::new(
                    "slice requires array or string",
                    Span::default(),
                )),
            },
            NativeId::ToString => Ok(VMValue::String(Rc::new(format!("{}", args[0])))),
            NativeId::Str => Ok(VMValue::String(Rc::new(format!("{}", args[0])))),
            NativeId::ToInt => match &args[0] {
                VMValue::Int(n) => Ok(VMValue::Int(*n)),
                VMValue::Float(n) => Ok(VMValue::Int(*n as i64)),
                VMValue::String(s) => s.parse::<i64>().map(VMValue::Int).map_err(|_| {
                    RuntimeError::new(format!("Cannot convert '{}' to Int", s), Span::default())
                }),
                VMValue::Bool(b) => Ok(VMValue::Int(if *b { 1 } else { 0 })),
                other => Err(RuntimeError::new(
                    format!("Cannot convert {} to Int", other.type_name()),
                    Span::default(),
                )),
            },
            NativeId::ToFloat => match &args[0] {
                VMValue::Int(n) => Ok(VMValue::Float(*n as f64)),
                VMValue::Float(n) => Ok(VMValue::Float(*n)),
                VMValue::String(s) => s.parse::<f64>().map(VMValue::Float).map_err(|_| {
                    RuntimeError::new(format!("Cannot convert '{}' to Float", s), Span::default())
                }),
                other => Err(RuntimeError::new(
                    format!("Cannot convert {} to Float", other.type_name()),
                    Span::default(),
                )),
            },
            NativeId::Upcase => {
                if let VMValue::String(s) = &args[0] {
                    Ok(VMValue::String(Rc::new(s.to_uppercase())))
                } else {
                    Err(RuntimeError::new(
                        "upcase requires a string",
                        Span::default(),
                    ))
                }
            }
            NativeId::Downcase => {
                if let VMValue::String(s) = &args[0] {
                    Ok(VMValue::String(Rc::new(s.to_lowercase())))
                } else {
                    Err(RuntimeError::new(
                        "downcase requires a string",
                        Span::default(),
                    ))
                }
            }
            NativeId::Trim => {
                if let VMValue::String(s) = &args[0] {
                    Ok(VMValue::String(Rc::new(s.trim().to_string())))
                } else {
                    Err(RuntimeError::new("trim requires a string", Span::default()))
                }
            }
            NativeId::Split => match (&args[0], &args[1]) {
                (VMValue::String(s), VMValue::String(delim)) => {
                    let parts: Vec<VMValue> = s
                        .split(delim.as_str())
                        .map(|p| VMValue::String(Rc::new(p.to_string())))
                        .collect();
                    Ok(VMValue::Array(Rc::new(RefCell::new(parts))))
                }
                _ => Err(RuntimeError::new(
                    "split requires (string, string)",
                    Span::default(),
                )),
            },
            NativeId::Join => match (&args[0], &args[1]) {
                (VMValue::Array(arr), VMValue::String(delim)) => {
                    let parts: Vec<String> =
                        arr.borrow().iter().map(|v| format!("{}", v)).collect();
                    Ok(VMValue::String(Rc::new(parts.join(delim.as_str()))))
                }
                _ => Err(RuntimeError::new(
                    "join requires (array, string)",
                    Span::default(),
                )),
            },
            NativeId::Contains => match (&args[0], &args[1]) {
                (VMValue::String(s), VMValue::String(sub)) => {
                    Ok(VMValue::Bool(s.contains(sub.as_str())))
                }
                (VMValue::Array(arr), val) => {
                    Ok(VMValue::Bool(arr.borrow().iter().any(|v| v == val)))
                }
                _ => Err(RuntimeError::new(
                    "contains requires (string, string) or (array, value)",
                    Span::default(),
                )),
            },
            NativeId::IndexOf => match (&args[0], &args[1]) {
                (VMValue::String(s), VMValue::String(sub)) => Ok(s
                    .find(sub.as_str())
                    .map(|i| VMValue::Int(i as i64))
                    .unwrap_or(VMValue::Int(-1))),
                (VMValue::Array(arr), val) => {
                    let arr = arr.borrow();
                    for (i, v) in arr.iter().enumerate() {
                        if v == val {
                            return Ok(VMValue::Int(i as i64));
                        }
                    }
                    Ok(VMValue::Int(-1))
                }
                _ => Err(RuntimeError::new(
                    "index_of requires (string, string) or (array, value)",
                    Span::default(),
                )),
            },
            NativeId::Substring => match (&args[0], &args[1], &args[2]) {
                (VMValue::String(s), VMValue::Int(start), VMValue::Int(len)) => {
                    let chars: Vec<char> = s.chars().collect();
                    let start = (*start as usize).min(chars.len());
                    let len = (*len as usize).min(chars.len() - start);
                    let sub: String = chars[start..start + len].iter().collect();
                    Ok(VMValue::String(Rc::new(sub)))
                }
                _ => Err(RuntimeError::new(
                    "substring requires (string, int, int)",
                    Span::default(),
                )),
            },
            NativeId::Map => match (&args[0], &args[1]) {
                (VMValue::Array(arr), VMValue::Closure(closure)) => {
                    let items: Vec<VMValue> = arr.borrow().clone();
                    let mut result = Vec::with_capacity(items.len());
                    for item in items {
                        let mapped = self.call_closure(closure.clone(), vec![item])?;
                        result.push(mapped);
                    }
                    Ok(VMValue::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err(RuntimeError::new(
                    "map requires (array, function)",
                    Span::default(),
                )),
            },
            NativeId::Filter => match (&args[0], &args[1]) {
                (VMValue::Array(arr), VMValue::Closure(closure)) => {
                    let items: Vec<VMValue> = arr.borrow().clone();
                    let mut result = Vec::new();
                    for item in items {
                        let keep = self.call_closure(closure.clone(), vec![item.clone()])?;
                        if keep.is_truthy() {
                            result.push(item);
                        }
                    }
                    Ok(VMValue::Array(Rc::new(RefCell::new(result))))
                }
                _ => Err(RuntimeError::new(
                    "filter requires (array, function)",
                    Span::default(),
                )),
            },
            NativeId::Fold => match (&args[0], &args[2]) {
                (VMValue::Array(arr), VMValue::Closure(closure)) => {
                    let items: Vec<VMValue> = arr.borrow().clone();
                    let mut acc = args[1].clone();
                    for item in items {
                        acc = self.call_closure(closure.clone(), vec![acc, item])?;
                    }
                    Ok(acc)
                }
                _ => Err(RuntimeError::new(
                    "fold requires (array, initial, function)",
                    Span::default(),
                )),
            },
            NativeId::Reverse => {
                if let VMValue::Array(arr) = &args[0] {
                    let mut reversed = arr.borrow().clone();
                    reversed.reverse();
                    Ok(VMValue::Array(Rc::new(RefCell::new(reversed))))
                } else {
                    Err(RuntimeError::new(
                        "reverse requires an array",
                        Span::default(),
                    ))
                }
            }
            NativeId::Sort => {
                if let VMValue::Array(arr) = &args[0] {
                    let mut sorted = arr.borrow().clone();
                    sorted.sort_by(|a, b| match (a, b) {
                        (VMValue::Int(x), VMValue::Int(y)) => x.cmp(y),
                        (VMValue::Float(x), VMValue::Float(y)) => {
                            x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                        }
                        (VMValue::String(x), VMValue::String(y)) => x.cmp(y),
                        _ => std::cmp::Ordering::Equal,
                    });
                    Ok(VMValue::Array(Rc::new(RefCell::new(sorted))))
                } else {
                    Err(RuntimeError::new("sort requires an array", Span::default()))
                }
            }
            NativeId::TypeOf => Ok(VMValue::String(Rc::new(args[0].type_name().to_string()))),
            NativeId::IsNull => Ok(VMValue::Bool(matches!(args[0], VMValue::Null))),
            NativeId::Now => {
                let duration = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                Ok(VMValue::Float(duration.as_secs_f64()))
            }
            NativeId::Clock => {
                let duration = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                Ok(VMValue::Float(duration.as_secs_f64()))
            }
            NativeId::Range => {
                match (&args[0], &args[1]) {
                    (VMValue::Int(start), VMValue::Int(end)) => {
                        let arr: Vec<VMValue> = (*start..*end).map(VMValue::Int).collect();
                        Ok(VMValue::Array(Rc::new(RefCell::new(arr))))
                    }
                    _ => Err(RuntimeError::new("range() expects two integers", Span::default())),
                }
            }
            NativeId::Abs => {
                match &args[0] {
                    VMValue::Int(n) => Ok(VMValue::Int(n.abs())),
                    VMValue::Float(n) => Ok(VMValue::Float(n.abs())),
                    _ => Err(RuntimeError::new("abs() expects a number", Span::default())),
                }
            }
            NativeId::Min => {
                match (&args[0], &args[1]) {
                    (VMValue::Int(a), VMValue::Int(b)) => Ok(VMValue::Int(*a.min(b))),
                    (VMValue::Float(a), VMValue::Float(b)) => Ok(VMValue::Float(a.min(*b))),
                    (VMValue::Int(a), VMValue::Float(b)) => Ok(VMValue::Float((*a as f64).min(*b))),
                    (VMValue::Float(a), VMValue::Int(b)) => Ok(VMValue::Float(a.min(*b as f64))),
                    _ => Err(RuntimeError::new("min() expects two numbers", Span::default())),
                }
            }
            NativeId::Max => {
                match (&args[0], &args[1]) {
                    (VMValue::Int(a), VMValue::Int(b)) => Ok(VMValue::Int(*a.max(b))),
                    (VMValue::Float(a), VMValue::Float(b)) => Ok(VMValue::Float(a.max(*b))),
                    (VMValue::Int(a), VMValue::Float(b)) => Ok(VMValue::Float((*a as f64).max(*b))),
                    (VMValue::Float(a), VMValue::Int(b)) => Ok(VMValue::Float(a.max(*b as f64))),
                    _ => Err(RuntimeError::new("max() expects two numbers", Span::default())),
                }
            }
            NativeId::Floor => {
                match &args[0] {
                    VMValue::Int(n) => Ok(VMValue::Int(*n)),
                    VMValue::Float(n) => Ok(VMValue::Float(n.floor())),
                    _ => Err(RuntimeError::new("floor() expects a number", Span::default())),
                }
            }
            NativeId::Ceil => {
                match &args[0] {
                    VMValue::Int(n) => Ok(VMValue::Int(*n)),
                    VMValue::Float(n) => Ok(VMValue::Float(n.ceil())),
                    _ => Err(RuntimeError::new("ceil() expects a number", Span::default())),
                }
            }
            NativeId::Round => {
                match &args[0] {
                    VMValue::Int(n) => Ok(VMValue::Int(*n)),
                    VMValue::Float(n) => Ok(VMValue::Float(n.round())),
                    _ => Err(RuntimeError::new("round() expects a number", Span::default())),
                }
            }
            NativeId::Sqrt => {
                match &args[0] {
                    VMValue::Int(n) => Ok(VMValue::Float((*n as f64).sqrt())),
                    VMValue::Float(n) => Ok(VMValue::Float(n.sqrt())),
                    _ => Err(RuntimeError::new("sqrt() expects a number", Span::default())),
                }
            }
            NativeId::Pow => {
                match (&args[0], &args[1]) {
                    (VMValue::Int(a), VMValue::Int(b)) => Ok(VMValue::Int(a.pow(*b as u32))),
                    (VMValue::Float(a), VMValue::Float(b)) => Ok(VMValue::Float(a.powf(*b))),
                    (VMValue::Int(a), VMValue::Float(b)) => Ok(VMValue::Float((*a as f64).powf(*b))),
                    (VMValue::Float(a), VMValue::Int(b)) => Ok(VMValue::Float(a.powf(*b as f64))),
                    _ => Err(RuntimeError::new("pow() expects two numbers", Span::default())),
                }
            }
            NativeId::Keys => {
                if let VMValue::Hash(hash) = &args[0] {
                    let keys: Vec<VMValue> = hash.borrow().iter().map(|(k, _)| k.clone()).collect();
                    Ok(VMValue::Array(Rc::new(RefCell::new(keys))))
                } else {
                    Err(RuntimeError::new("keys requires a hash", Span::default()))
                }
            }
            NativeId::Values => {
                if let VMValue::Hash(hash) = &args[0] {
                    let values: Vec<VMValue> =
                        hash.borrow().iter().map(|(_, v)| v.clone()).collect();
                    Ok(VMValue::Array(Rc::new(RefCell::new(values))))
                } else {
                    Err(RuntimeError::new("values requires a hash", Span::default()))
                }
            }
            NativeId::Entries => {
                if let VMValue::Hash(hash) = &args[0] {
                    let pairs: Vec<VMValue> = hash
                        .borrow()
                        .iter()
                        .map(|(k, v)| {
                            VMValue::Array(Rc::new(RefCell::new(vec![k.clone(), v.clone()])))
                        })
                        .collect();
                    Ok(VMValue::Array(Rc::new(RefCell::new(pairs))))
                } else {
                    Err(RuntimeError::new(
                        "entries requires a hash",
                        Span::default(),
                    ))
                }
            }
            NativeId::FromEntries => {
                if let VMValue::Array(arr) = &args[0] {
                    let mut result: Vec<(VMValue, VMValue)> = Vec::new();
                    for entry in arr.borrow().iter() {
                        if let VMValue::Array(pair) = entry {
                            let borrowed = pair.borrow();
                            if borrowed.len() != 2 {
                                return Err(RuntimeError::new(
                                    "from_entries expects array of [key, value] pairs",
                                    Span::default(),
                                ));
                            }
                            let key = &borrowed[0];
                            // Update existing key or add new one
                            let mut found = false;
                            for (k, v) in result.iter_mut() {
                                if k == key {
                                    *v = borrowed[1].clone();
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                result.push((key.clone(), borrowed[1].clone()));
                            }
                        } else {
                            return Err(RuntimeError::new(
                                "from_entries expects array of [key, value] pairs",
                                Span::default(),
                            ));
                        }
                    }
                    Ok(VMValue::Hash(Rc::new(RefCell::new(result))))
                } else {
                    Err(RuntimeError::new(
                        "from_entries requires an array",
                        Span::default(),
                    ))
                }
            }
            NativeId::HasKey => match (&args[0], &args[1]) {
                (VMValue::Hash(hash), key) => {
                    let exists = hash.borrow().iter().any(|(k, _)| k == key);
                    Ok(VMValue::Bool(exists))
                }
                _ => Err(RuntimeError::new(
                    "has_key requires (hash, key)",
                    Span::default(),
                )),
            },
            NativeId::Delete => match (&args[0], &args[1]) {
                (VMValue::Hash(hash), key) => {
                    let mut hash = hash.borrow_mut();
                    let removed = hash.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
                    hash.retain(|(k, _)| k != key);
                    Ok(removed.unwrap_or(VMValue::Null))
                }
                _ => Err(RuntimeError::new(
                    "delete requires (hash, key)",
                    Span::default(),
                )),
            },
            NativeId::Merge => match (&args[0], &args[1]) {
                (VMValue::Hash(hash1), VMValue::Hash(hash2)) => {
                    let mut result: Vec<(VMValue, VMValue)> = hash1.borrow().clone();
                    for (k2, v2) in hash2.borrow().iter() {
                        let mut found = false;
                        for (k1, v1) in result.iter_mut() {
                            if k1 == k2 {
                                *v1 = v2.clone();
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            result.push((k2.clone(), v2.clone()));
                        }
                    }
                    Ok(VMValue::Hash(Rc::new(RefCell::new(result))))
                }
                _ => Err(RuntimeError::new(
                    "merge requires (hash, hash)",
                    Span::default(),
                )),
            },
            NativeId::Clear => match &args[0] {
                VMValue::Hash(hash) => {
                    hash.borrow_mut().clear();
                    Ok(VMValue::Null)
                }
                VMValue::Array(arr) => {
                    arr.borrow_mut().clear();
                    Ok(VMValue::Null)
                }
                _ => Err(RuntimeError::new(
                    "clear requires hash or array",
                    Span::default(),
                )),
            },

            // HTTP functions
            NativeId::HttpGet => {
                let url = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "http_get requires a string URL",
                            Span::default(),
                        ))
                    }
                };
                match ureq::get(url).call() {
                    Ok(response) => {
                        let body = response.into_string().map_err(|e| {
                            RuntimeError::new(
                                format!("Failed to read response: {}", e),
                                Span::default(),
                            )
                        })?;
                        Ok(VMValue::String(Rc::new(body)))
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();
                        Err(RuntimeError::new(
                            format!("HTTP {} error: {}", code, body),
                            Span::default(),
                        ))
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("HTTP request failed: {}", e),
                        Span::default(),
                    )),
                }
            }

            NativeId::HttpPost => {
                let url = match &args[0] {
                    VMValue::String(s) => s.to_string(),
                    _ => {
                        return Err(RuntimeError::new(
                            "http_post requires a string URL",
                            Span::default(),
                        ))
                    }
                };
                let body = match &args[1] {
                    VMValue::String(s) => s.to_string(),
                    VMValue::Hash(_) => vm_value_to_json(&args[1])?,
                    _ => {
                        return Err(RuntimeError::new(
                            "http_post body must be string or hash",
                            Span::default(),
                        ))
                    }
                };
                let content_type = if matches!(args[1], VMValue::Hash(_)) {
                    "application/json"
                } else {
                    "text/plain"
                };
                match ureq::post(&url)
                    .set("Content-Type", content_type)
                    .send_string(&body)
                {
                    Ok(response) => {
                        let body = response.into_string().map_err(|e| {
                            RuntimeError::new(
                                format!("Failed to read response: {}", e),
                                Span::default(),
                            )
                        })?;
                        Ok(VMValue::String(Rc::new(body)))
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();
                        Err(RuntimeError::new(
                            format!("HTTP {} error: {}", code, body),
                            Span::default(),
                        ))
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("HTTP request failed: {}", e),
                        Span::default(),
                    )),
                }
            }

            NativeId::HttpGetJson => {
                let url = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "http_get_json requires a string URL",
                            Span::default(),
                        ))
                    }
                };
                match ureq::get(url).set("Accept", "application/json").call() {
                    Ok(response) => {
                        let body = response.into_string().map_err(|e| {
                            RuntimeError::new(
                                format!("Failed to read response: {}", e),
                                Span::default(),
                            )
                        })?;
                        match serde_json::from_str::<serde_json::Value>(&body) {
                            Ok(json) => json_to_vm_value(&json),
                            Err(e) => Err(RuntimeError::new(
                                format!("Failed to parse JSON: {}", e),
                                Span::default(),
                            )),
                        }
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();
                        Err(RuntimeError::new(
                            format!("HTTP {} error: {}", code, body),
                            Span::default(),
                        ))
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("HTTP request failed: {}", e),
                        Span::default(),
                    )),
                }
            }

            NativeId::HttpPostJson => {
                let url = match &args[0] {
                    VMValue::String(s) => s.to_string(),
                    _ => {
                        return Err(RuntimeError::new(
                            "http_post_json requires a string URL",
                            Span::default(),
                        ))
                    }
                };
                let json_body = vm_value_to_json(&args[1])?;
                match ureq::post(&url)
                    .set("Content-Type", "application/json")
                    .send_string(&json_body)
                {
                    Ok(response) => {
                        let body = response.into_string().map_err(|e| {
                            RuntimeError::new(
                                format!("Failed to read response: {}", e),
                                Span::default(),
                            )
                        })?;
                        match serde_json::from_str::<serde_json::Value>(&body) {
                            Ok(json) => json_to_vm_value(&json),
                            Err(_) => Ok(VMValue::String(Rc::new(body))),
                        }
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();
                        Err(RuntimeError::new(
                            format!("HTTP {} error: {}", code, body),
                            Span::default(),
                        ))
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("HTTP request failed: {}", e),
                        Span::default(),
                    )),
                }
            }

            NativeId::HttpRequest => {
                if args.len() < 2 {
                    return Err(RuntimeError::new(
                        "http_request requires at least method and URL",
                        Span::default(),
                    ));
                }
                let method = match &args[0] {
                    VMValue::String(s) => s.to_uppercase(),
                    _ => {
                        return Err(RuntimeError::new(
                            "http_request method must be string",
                            Span::default(),
                        ))
                    }
                };
                let url = match &args[1] {
                    VMValue::String(s) => s.to_string(),
                    _ => {
                        return Err(RuntimeError::new(
                            "http_request URL must be string",
                            Span::default(),
                        ))
                    }
                };
                let mut request = match method.as_str() {
                    "GET" => ureq::get(&url),
                    "POST" => ureq::post(&url),
                    "PUT" => ureq::put(&url),
                    "DELETE" => ureq::delete(&url),
                    "PATCH" => ureq::patch(&url),
                    "HEAD" => ureq::head(&url),
                    _ => {
                        return Err(RuntimeError::new(
                            format!("Unsupported HTTP method: {}", method),
                            Span::default(),
                        ))
                    }
                };
                // Add headers if provided
                if args.len() > 2 {
                    if let VMValue::Hash(headers) = &args[2] {
                        for (k, v) in headers.borrow().iter() {
                            if let (VMValue::String(key), VMValue::String(val)) = (k, v) {
                                request = request.set(key.as_str(), val.as_str());
                            }
                        }
                    }
                }
                // Send request with optional body
                let response = if args.len() > 3 {
                    let body = match &args[3] {
                        VMValue::String(s) => s.to_string(),
                        VMValue::Hash(_) => vm_value_to_json(&args[3])?,
                        VMValue::Null => String::new(),
                        other => format!("{}", other),
                    };
                    request.send_string(&body)
                } else {
                    request.call()
                };
                // Build response hash
                match response {
                    Ok(resp) => {
                        let status = resp.status();
                        let status_text = resp.status_text().to_string();
                        let mut headers: Vec<(VMValue, VMValue)> = Vec::new();
                        for name in resp.headers_names() {
                            if let Some(value) = resp.header(&name) {
                                headers.push((
                                    VMValue::String(Rc::new(name)),
                                    VMValue::String(Rc::new(value.to_string())),
                                ));
                            }
                        }
                        let body = resp.into_string().unwrap_or_default();
                        let result: Vec<(VMValue, VMValue)> = vec![
                            (
                                VMValue::String(Rc::new("status".to_string())),
                                VMValue::Int(status as i64),
                            ),
                            (
                                VMValue::String(Rc::new("status_text".to_string())),
                                VMValue::String(Rc::new(status_text)),
                            ),
                            (
                                VMValue::String(Rc::new("headers".to_string())),
                                VMValue::Hash(Rc::new(RefCell::new(headers))),
                            ),
                            (
                                VMValue::String(Rc::new("body".to_string())),
                                VMValue::String(Rc::new(body)),
                            ),
                        ];
                        Ok(VMValue::Hash(Rc::new(RefCell::new(result))))
                    }
                    Err(ureq::Error::Status(code, resp)) => {
                        let body = resp.into_string().unwrap_or_default();
                        let result: Vec<(VMValue, VMValue)> = vec![
                            (
                                VMValue::String(Rc::new("status".to_string())),
                                VMValue::Int(code as i64),
                            ),
                            (
                                VMValue::String(Rc::new("status_text".to_string())),
                                VMValue::String(Rc::new("Error".to_string())),
                            ),
                            (
                                VMValue::String(Rc::new("headers".to_string())),
                                VMValue::Hash(Rc::new(RefCell::new(vec![]))),
                            ),
                            (
                                VMValue::String(Rc::new("body".to_string())),
                                VMValue::String(Rc::new(body)),
                            ),
                        ];
                        Ok(VMValue::Hash(Rc::new(RefCell::new(result))))
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("HTTP request failed: {}", e),
                        Span::default(),
                    )),
                }
            }

            NativeId::JsonParse => {
                let json_str = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "json_parse requires a string",
                            Span::default(),
                        ))
                    }
                };
                match serde_json::from_str::<serde_json::Value>(json_str) {
                    Ok(json) => json_to_vm_value(&json),
                    Err(e) => Err(RuntimeError::new(
                        format!("Failed to parse JSON: {}", e),
                        Span::default(),
                    )),
                }
            }

            NativeId::JsonStringify => {
                let json_str = vm_value_to_json(&args[0])?;
                Ok(VMValue::String(Rc::new(json_str)))
            }
            NativeId::HttpOk => {
                let status = extract_vm_status(&args[0])?;
                Ok(VMValue::Bool((200..300).contains(&status)))
            }
            NativeId::HttpSuccess => {
                let status = extract_vm_status(&args[0])?;
                Ok(VMValue::Bool((200..300).contains(&status)))
            }
            NativeId::HttpRedirect => {
                let status = extract_vm_status(&args[0])?;
                Ok(VMValue::Bool((300..400).contains(&status)))
            }
            NativeId::HttpClientError => {
                let status = extract_vm_status(&args[0])?;
                Ok(VMValue::Bool((400..500).contains(&status)))
            }
            NativeId::HttpServerError => {
                let status = extract_vm_status(&args[0])?;
                Ok(VMValue::Bool((500..600).contains(&status)))
            }
            NativeId::Barf => {
                // barf(path, content) or barf(path, bytes)
                // args[0] = path, args[1] = content (string or array)
                let path = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "barf requires string path",
                            Span::default(),
                        ))
                    }
                };

                match &args[1] {
                    VMValue::String(content) => {
                        std::fs::write(path, content.as_str()).map_err(|e| {
                            RuntimeError::new(
                                format!("barf failed to write {}: {}", path, e),
                                Span::default(),
                            )
                        })?;
                        Ok(VMValue::Null)
                    }
                    VMValue::Array(bytes) => {
                        let byte_vec: Result<Vec<u8>, RuntimeError> = bytes
                            .borrow()
                            .iter()
                            .map(|b| match b {
                                VMValue::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                                VMValue::Int(n) => Err(RuntimeError::new(
                                    format!("byte value {} out of range", n),
                                    Span::default(),
                                )),
                                other => Err(RuntimeError::new(
                                    format!("expected byte, got {}", other.type_name()),
                                    Span::default(),
                                )),
                            })
                            .collect();
                        std::fs::write(path, byte_vec?).map_err(|e| {
                            RuntimeError::new(
                                format!("barf failed to write {}: {}", path, e),
                                Span::default(),
                            )
                        })?;
                        Ok(VMValue::Null)
                    }
                    _ => Err(RuntimeError::new(
                        "barf expects (string, string) or (string, array<int>)",
                        Span::default(),
                    )),
                }
            }
            NativeId::Slurp => {
                // slurp(path) or slurp(path, mode)
                let path = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "slurp requires string path",
                            Span::default(),
                        ))
                    }
                };

                if args.len() == 1 {
                    // Text mode (default)
                    let content = std::fs::read_to_string(path).map_err(|e| {
                        RuntimeError::new(
                            format!("slurp failed to read {}: {}", path, e),
                            Span::default(),
                        )
                    })?;
                    Ok(VMValue::String(Rc::new(content)))
                } else {
                    // Check mode argument
                    let mode = match &args[1] {
                        VMValue::String(s) => s.as_str(),
                        _ => {
                            return Err(RuntimeError::new(
                                "slurp mode must be string",
                                Span::default(),
                            ))
                        }
                    };

                    if mode == "binary" {
                        let bytes = std::fs::read(path).map_err(|e| {
                            RuntimeError::new(
                                format!("slurp failed to read {}: {}", path, e),
                                Span::default(),
                            )
                        })?;
                        let value_bytes: Vec<VMValue> =
                            bytes.iter().map(|&b| VMValue::Int(b as i64)).collect();
                        Ok(VMValue::Array(Rc::new(RefCell::new(value_bytes))))
                    } else {
                        let content = std::fs::read_to_string(path).map_err(|e| {
                            RuntimeError::new(
                                format!("slurp failed to read {}: {}", path, e),
                                Span::default(),
                            )
                        })?;
                        Ok(VMValue::String(Rc::new(content)))
                    }
                }
            }
            NativeId::HtmlEscape => {
                let s = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "html_escape requires a string",
                            Span::default(),
                        ))
                    }
                };
                let escaped = s
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&#39;");
                Ok(VMValue::String(Rc::new(escaped)))
            }
            NativeId::HtmlUnescape => {
                let s = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "html_unescape requires a string",
                            Span::default(),
                        ))
                    }
                };
                let mut result = s.to_string();
                let replacements = [
                    ("&amp;", "&"),
                    ("&lt;", "<"),
                    ("&gt;", ">"),
                    ("&quot;", "\""),
                    ("&#39;", "'"),
                    ("&apos;", "'"),
                    ("&nbsp;", " "),
                ];
                for (from, to) in replacements {
                    result = result.replace(from, to);
                }
                Ok(VMValue::String(Rc::new(result)))
            }
            NativeId::SanitizeHtml => {
                let s = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "sanitize_html requires a string",
                            Span::default(),
                        ))
                    }
                };
                let mut result = String::new();
                let mut in_tag = false;
                let mut current_tag = String::new();
                let mut tag_buffer = String::new();
                let mut chars = s.chars().peekable();

                while let Some(c) = chars.next() {
                    if c == '<' {
                        if in_tag {
                            result.push_str(&current_tag);
                        }
                        in_tag = true;
                        current_tag = String::new();
                        tag_buffer.clear();
                        tag_buffer.push(c);
                    } else if c == '>' {
                        if in_tag {
                            tag_buffer.push(c);
                            let tag = tag_buffer.trim().to_lowercase();
                            let is_closing = tag.starts_with("</");
                            let is_self_closing = tag.ends_with("/>")
                                || matches!(
                                    tag.strip_suffix(">").and_then(|t| t.strip_suffix('/')),
                                    Some(_)
                                );
                            let tag_name = if is_closing {
                                tag.trim_start_matches('<')
                                    .trim_start_matches('/')
                                    .trim_end_matches('>')
                                    .split_whitespace()
                                    .next()
                                    .unwrap_or("")
                            } else {
                                tag.trim_start_matches('<')
                                    .trim_end_matches('/')
                                    .trim_end_matches('>')
                                    .split_whitespace()
                                    .next()
                                    .unwrap_or("")
                            };
                            let allowed_tags = [
                                "p",
                                "br",
                                "b",
                                "i",
                                "u",
                                "em",
                                "strong",
                                "a",
                                "ul",
                                "ol",
                                "li",
                                "blockquote",
                                "code",
                                "pre",
                                "h1",
                                "h2",
                                "h3",
                                "h4",
                                "h5",
                                "h6",
                                "span",
                                "div",
                                "img",
                            ];
                            let is_allowed = allowed_tags.contains(&tag_name);
                            let _is_script = tag_name == "script"
                                || tag_name == "style"
                                || tag_name == "iframe"
                                || tag_name == "object"
                                || tag_name == "embed"
                                || tag_name == "form"
                                || tag_name == "input";
                            let is_dangerous_attr = tag.contains("javascript:")
                                || tag.contains("onload=")
                                || tag.contains("onerror=")
                                || tag.contains("onclick=");
                            if is_allowed && !is_dangerous_attr {
                                let cleaned_tag = if is_closing {
                                    format!("</{}>", tag_name)
                                } else if is_self_closing {
                                    format!("<{}/>", tag_name)
                                } else {
                                    let attrs: Vec<&str> = tag
                                        .strip_prefix('<')
                                        .and_then(|s| s.strip_suffix('>').or(Some(s)))
                                        .unwrap_or("")
                                        .split_whitespace()
                                        .skip(1)
                                        .collect();
                                    let safe_attrs: Vec<String> = attrs
                                        .iter()
                                        .filter_map(|&attr| {
                                            let parts: Vec<&str> = attr.splitn(2, '=').collect();
                                            if parts.len() == 2 {
                                                let attr_name = parts[0].to_lowercase();
                                                let attr_value =
                                                    parts[1].trim_matches('"').trim_matches('\'');
                                                let safe_attrs = [
                                                    "href", "src", "title", "alt", "class", "id",
                                                    "style",
                                                ];
                                                if safe_attrs.contains(&attr_name.as_str())
                                                    && !attr_value
                                                        .to_lowercase()
                                                        .contains("javascript:")
                                                {
                                                    Some(format!("{}={}", attr_name, parts[1]))
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    if safe_attrs.is_empty() {
                                        format!("<{}>", tag_name)
                                    } else {
                                        format!("<{} {}>", tag_name, safe_attrs.join(" "))
                                    }
                                };
                                result.push_str(&cleaned_tag);
                            }
                            in_tag = false;
                            current_tag.clear();
                        } else {
                            result.push(c);
                        }
                    } else if in_tag {
                        tag_buffer.push(c);
                    } else {
                        result.push(c);
                    }
                }
                if in_tag {
                    result.push_str(&current_tag);
                }
                Ok(VMValue::String(Rc::new(result)))
            }
            NativeId::StripHtml => {
                let s = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "strip_html requires a string",
                            Span::default(),
                        ))
                    }
                };
                let mut result = String::new();
                let mut in_tag = false;

                for c in s.chars() {
                    if c == '<' {
                        in_tag = true;
                    } else if c == '>' {
                        in_tag = false;
                    } else if !in_tag {
                        result.push(c);
                    }
                }
                Ok(VMValue::String(Rc::new(result)))
            }
            // Regex functions
            NativeId::RegexMatch => match (&args[0], &args[1]) {
                (VMValue::String(pattern), VMValue::String(s)) => match Regex::new(pattern) {
                    Ok(re) => Ok(VMValue::Bool(re.is_match(s))),
                    Err(e) => Err(RuntimeError::new(
                        format!("Invalid regex pattern: {}", e),
                        Span::default(),
                    )),
                },
                _ => Err(RuntimeError::new(
                    "regex_match requires (string, string)",
                    Span::default(),
                )),
            },
            NativeId::RegexFind => match (&args[0], &args[1]) {
                (VMValue::String(pattern), VMValue::String(s)) => match Regex::new(pattern) {
                    Ok(re) => {
                        if let Some(m) = re.find(s) {
                            let matches: Vec<(VMValue, VMValue)> = vec![
                                (
                                    VMValue::String(Rc::new("match".to_string())),
                                    VMValue::String(Rc::new(m.as_str().to_string())),
                                ),
                                (
                                    VMValue::String(Rc::new("start".to_string())),
                                    VMValue::Int(m.start() as i64),
                                ),
                                (
                                    VMValue::String(Rc::new("end".to_string())),
                                    VMValue::Int(m.end() as i64),
                                ),
                            ];
                            Ok(VMValue::Hash(Rc::new(RefCell::new(matches))))
                        } else {
                            Ok(VMValue::Null)
                        }
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("Invalid regex pattern: {}", e),
                        Span::default(),
                    )),
                },
                _ => Err(RuntimeError::new(
                    "regex_find requires (string, string)",
                    Span::default(),
                )),
            },
            NativeId::RegexFindAll => match (&args[0], &args[1]) {
                (VMValue::String(pattern), VMValue::String(s)) => match Regex::new(pattern) {
                    Ok(re) => {
                        let matches: Vec<VMValue> = re
                            .find_iter(s)
                            .map(|m| {
                                let match_hash: Vec<(VMValue, VMValue)> = vec![
                                    (
                                        VMValue::String(Rc::new("match".to_string())),
                                        VMValue::String(Rc::new(m.as_str().to_string())),
                                    ),
                                    (
                                        VMValue::String(Rc::new("start".to_string())),
                                        VMValue::Int(m.start() as i64),
                                    ),
                                    (
                                        VMValue::String(Rc::new("end".to_string())),
                                        VMValue::Int(m.end() as i64),
                                    ),
                                ];
                                VMValue::Hash(Rc::new(RefCell::new(match_hash)))
                            })
                            .collect();
                        Ok(VMValue::Array(Rc::new(RefCell::new(matches))))
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("Invalid regex pattern: {}", e),
                        Span::default(),
                    )),
                },
                _ => Err(RuntimeError::new(
                    "regex_find_all requires (string, string)",
                    Span::default(),
                )),
            },
            NativeId::RegexReplace => match (&args[0], &args[1], &args[2]) {
                (VMValue::String(pattern), VMValue::String(s), VMValue::String(replacement)) => {
                    match Regex::new(pattern) {
                        Ok(re) => {
                            let result = re.replace(s, replacement.as_str());
                            Ok(VMValue::String(Rc::new(result.to_string())))
                        }
                        Err(e) => Err(RuntimeError::new(
                            format!("Invalid regex pattern: {}", e),
                            Span::default(),
                        )),
                    }
                }
                _ => Err(RuntimeError::new(
                    "regex_replace requires (string, string, string)",
                    Span::default(),
                )),
            },
            NativeId::RegexReplaceAll => match (&args[0], &args[1], &args[2]) {
                (VMValue::String(pattern), VMValue::String(s), VMValue::String(replacement)) => {
                    match Regex::new(pattern) {
                        Ok(re) => {
                            let result = re.replace_all(s, replacement.as_str());
                            Ok(VMValue::String(Rc::new(result.to_string())))
                        }
                        Err(e) => Err(RuntimeError::new(
                            format!("Invalid regex pattern: {}", e),
                            Span::default(),
                        )),
                    }
                }
                _ => Err(RuntimeError::new(
                    "regex_replace_all requires (string, string, string)",
                    Span::default(),
                )),
            },
            NativeId::RegexSplit => match (&args[0], &args[1]) {
                (VMValue::String(pattern), VMValue::String(s)) => match Regex::new(pattern) {
                    Ok(re) => {
                        let parts: Vec<VMValue> = re
                            .split(s)
                            .map(|p| VMValue::String(Rc::new(p.to_string())))
                            .collect();
                        Ok(VMValue::Array(Rc::new(RefCell::new(parts))))
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("Invalid regex pattern: {}", e),
                        Span::default(),
                    )),
                },
                _ => Err(RuntimeError::new(
                    "regex_split requires (string, string)",
                    Span::default(),
                )),
            },
            NativeId::RegexCapture => match (&args[0], &args[1]) {
                (VMValue::String(pattern), VMValue::String(s)) => match Regex::new(pattern) {
                    Ok(re) => {
                        if let Some(caps) = re.captures(s) {
                            let mut result: Vec<(VMValue, VMValue)> = vec![
                                (
                                    VMValue::String(Rc::new("match".to_string())),
                                    VMValue::String(Rc::new(
                                        caps.get(0)
                                            .map(|m| m.as_str().to_string())
                                            .unwrap_or_default(),
                                    )),
                                ),
                                (
                                    VMValue::String(Rc::new("start".to_string())),
                                    VMValue::Int(
                                        caps.get(0).map(|m| m.start() as i64).unwrap_or(-1),
                                    ),
                                ),
                                (
                                    VMValue::String(Rc::new("end".to_string())),
                                    VMValue::Int(caps.get(0).map(|m| m.end() as i64).unwrap_or(-1)),
                                ),
                            ];
                            for name in re.capture_names().flatten() {
                                if let Some(cap) = caps.name(name) {
                                    result.push((
                                        VMValue::String(Rc::new(name.to_string())),
                                        VMValue::String(Rc::new(cap.as_str().to_string())),
                                    ));
                                }
                            }
                            Ok(VMValue::Hash(Rc::new(RefCell::new(result))))
                        } else {
                            Ok(VMValue::Null)
                        }
                    }
                    Err(e) => Err(RuntimeError::new(
                        format!("Invalid regex pattern: {}", e),
                        Span::default(),
                    )),
                },
                _ => Err(RuntimeError::new(
                    "regex_capture requires (string, string)",
                    Span::default(),
                )),
            },
            NativeId::RegexEscape => match &args[0] {
                VMValue::String(s) => {
                    let escaped = regex::escape(s);
                    Ok(VMValue::String(Rc::new(escaped)))
                }
                _ => Err(RuntimeError::new(
                    "regex_escape requires (string)",
                    Span::default(),
                )),
            },
            // DateTime functions
            NativeId::DateTimeNow => {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                let timestamp = now.as_secs() as i64;
                let class = Rc::new(RefCell::new(VMClass::new("DateTime".to_string())));
                let instance =
                    VMInstance::with_field(class, "timestamp".to_string(), VMValue::Int(timestamp));
                Ok(VMValue::Instance(Rc::new(RefCell::new(instance))))
            }
            NativeId::DateTimeParse => {
                let s = match &args[0] {
                    VMValue::String(s) => s.as_str(),
                    _ => {
                        return Err(RuntimeError::new(
                            "datetime_parse requires a string",
                            Span::default(),
                        ))
                    }
                };
                let timestamp =
                    parse_datetime_string(s).map_err(|e| RuntimeError::new(e, Span::default()))?;
                let class = Rc::new(RefCell::new(VMClass::new("DateTime".to_string())));
                let instance =
                    VMInstance::with_field(class, "timestamp".to_string(), VMValue::Int(timestamp));
                Ok(VMValue::Instance(Rc::new(RefCell::new(instance))))
            }
            NativeId::DateTimeUtc => {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                let timestamp = now.as_secs() as i64;
                let class = Rc::new(RefCell::new(VMClass::new("DateTime".to_string())));
                let instance =
                    VMInstance::with_field(class, "timestamp".to_string(), VMValue::Int(timestamp));
                Ok(VMValue::Instance(Rc::new(RefCell::new(instance))))
            }
            NativeId::DurationBetween => {
                let (start, end) = match (&args[0], &args[1]) {
                    (VMValue::Instance(i1), VMValue::Instance(i2)) => {
                        let ts1 = i1.borrow().get("timestamp");
                        let ts2 = i2.borrow().get("timestamp");
                        match (ts1, ts2) {
                            (Some(VMValue::Int(t1)), Some(VMValue::Int(t2))) => (t1, t2),
                            _ => return Err(RuntimeError::new(
                                "duration_between requires DateTime instances with timestamp field",
                                Span::default(),
                            )),
                        }
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "duration_between requires two DateTime instances",
                            Span::default(),
                        ))
                    }
                };
                let class = Rc::new(RefCell::new(VMClass::new("Duration".to_string())));
                let instance =
                    VMInstance::with_field(class, "seconds".to_string(), VMValue::Int(end - start));
                Ok(VMValue::Instance(Rc::new(RefCell::new(instance))))
            }
            NativeId::DurationSeconds => {
                let seconds = match &args[0] {
                    VMValue::Instance(inst) => inst
                        .borrow()
                        .get("seconds")
                        .or_else(|| inst.borrow().get("timestamp"))
                        .ok_or_else(|| {
                            RuntimeError::new(
                                "duration_seconds requires a Duration or DateTime instance",
                                Span::default(),
                            )
                        })?,
                    _ => {
                        return Err(RuntimeError::new(
                            "duration_seconds requires a Duration instance",
                            Span::default(),
                        ))
                    }
                };
                match seconds {
                    VMValue::Int(s) => Ok(VMValue::Float(s as f64)),
                    VMValue::Float(s) => Ok(VMValue::Float(s)),
                    _ => Err(RuntimeError::new("Invalid duration value", Span::default())),
                }
            }
            NativeId::DurationMinutes => {
                let seconds = match &args[0] {
                    VMValue::Instance(inst) => inst
                        .borrow()
                        .get("seconds")
                        .or_else(|| inst.borrow().get("timestamp"))
                        .ok_or_else(|| {
                            RuntimeError::new(
                                "duration_minutes requires a Duration or DateTime instance",
                                Span::default(),
                            )
                        })?,
                    _ => {
                        return Err(RuntimeError::new(
                            "duration_minutes requires a Duration instance",
                            Span::default(),
                        ))
                    }
                };
                match seconds {
                    VMValue::Int(s) => Ok(VMValue::Float(s as f64 / 60.0)),
                    VMValue::Float(s) => Ok(VMValue::Float(s / 60.0)),
                    _ => Err(RuntimeError::new("Invalid duration value", Span::default())),
                }
            }
            NativeId::DurationHours => {
                let seconds = match &args[0] {
                    VMValue::Instance(inst) => inst
                        .borrow()
                        .get("seconds")
                        .or_else(|| inst.borrow().get("timestamp"))
                        .ok_or_else(|| {
                            RuntimeError::new(
                                "duration_hours requires a Duration or DateTime instance",
                                Span::default(),
                            )
                        })?,
                    _ => {
                        return Err(RuntimeError::new(
                            "duration_hours requires a Duration instance",
                            Span::default(),
                        ))
                    }
                };
                match seconds {
                    VMValue::Int(s) => Ok(VMValue::Float(s as f64 / 3600.0)),
                    VMValue::Float(s) => Ok(VMValue::Float(s / 3600.0)),
                    _ => Err(RuntimeError::new("Invalid duration value", Span::default())),
                }
            }
            NativeId::DurationDays => {
                let seconds = match &args[0] {
                    VMValue::Instance(inst) => inst
                        .borrow()
                        .get("seconds")
                        .or_else(|| inst.borrow().get("timestamp"))
                        .ok_or_else(|| {
                            RuntimeError::new(
                                "duration_days requires a Duration or DateTime instance",
                                Span::default(),
                            )
                        })?,
                    _ => {
                        return Err(RuntimeError::new(
                            "duration_days requires a Duration instance",
                            Span::default(),
                        ))
                    }
                };
                match seconds {
                    VMValue::Int(s) => Ok(VMValue::Float(s as f64 / 86400.0)),
                    VMValue::Float(s) => Ok(VMValue::Float(s / 86400.0)),
                    _ => Err(RuntimeError::new("Invalid duration value", Span::default())),
                }
            }
            NativeId::DurationWeeks => {
                let seconds = match &args[0] {
                    VMValue::Instance(inst) => inst
                        .borrow()
                        .get("seconds")
                        .or_else(|| inst.borrow().get("timestamp"))
                        .ok_or_else(|| {
                            RuntimeError::new(
                                "duration_weeks requires a Duration or DateTime instance",
                                Span::default(),
                            )
                        })?,
                    _ => {
                        return Err(RuntimeError::new(
                            "duration_weeks requires a Duration instance",
                            Span::default(),
                        ))
                    }
                };
                match seconds {
                    VMValue::Int(s) => Ok(VMValue::Float(s as f64 / 604800.0)),
                    VMValue::Float(s) => Ok(VMValue::Float(s / 604800.0)),
                    _ => Err(RuntimeError::new("Invalid duration value", Span::default())),
                }
            }
            NativeId::Await => {
                // For non-Future values, just return as-is
                // Futures in the VM are not currently supported, so this is mostly a pass-through
                Ok(args.into_iter().next().unwrap_or(VMValue::Null))
            }
        }
    }

    /// Run a compiled function.
    pub fn run(&mut self, function: CompiledFunction) -> VMResult<()> {
        // Reset execution state (but preserve globals for REPL)
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();
        self.exception_handlers.clear();
        self.current_exception = None;

        // Wrap the function in a closure
        let closure = Rc::new(RefCell::new(Closure::new(Rc::new(function))));

        // Push closure onto stack (slot 0 for the top-level frame)
        self.push(VMValue::Closure(closure.clone()));

        // Push initial frame
        self.frames.push(CallFrame {
            closure,
            ip: 0,
            slots_start: 0,
        });

        self.execute()
    }

    /// Main execution loop.
    fn execute(&mut self) -> VMResult<()> {
        loop {
            if self.frames.is_empty() {
                return Ok(());
            }

            let op = self.read_byte();
            let opcode = OpCode::from_u8(op).ok_or_else(|| {
                RuntimeError::new(format!("Invalid opcode: {}", op), Span::default())
            })?;

            match opcode {
                OpCode::Constant => {
                    let idx = self.read_u16();
                    let value = self.read_constant(idx)?;
                    self.push(value);
                }

                OpCode::Null => self.push(VMValue::Null),
                OpCode::True => self.push(VMValue::Bool(true)),
                OpCode::False => self.push(VMValue::Bool(false)),

                OpCode::Pop => {
                    self.pop()?;
                }

                OpCode::Dup => {
                    let value = self.peek(0)?.clone();
                    self.push(value);
                }

                OpCode::GetLocal => {
                    let slot = self.read_u16() as usize;
                    let slots_start = self.current_frame().slots_start;
                    // +1 because slot 0 is the closure itself
                    let value = self.stack[slots_start + 1 + slot].clone();
                    self.push(value);
                }

                OpCode::SetLocal => {
                    let slot = self.read_u16() as usize;
                    let slots_start = self.current_frame().slots_start;
                    let value = self.peek(0)?.clone();
                    // +1 because slot 0 is the closure itself
                    self.stack[slots_start + 1 + slot] = value;
                }

                OpCode::GetGlobal => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    let value = self.globals.get(&name).cloned().ok_or_else(|| {
                        RuntimeError::new(format!("Undefined variable '{}'", name), Span::default())
                    })?;
                    self.push(value);
                }

                OpCode::SetGlobal => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    if !self.globals.contains_key(&name) {
                        return Err(RuntimeError::new(
                            format!("Undefined variable '{}'", name),
                            Span::default(),
                        ));
                    }
                    let value = self.peek(0)?.clone();
                    self.globals.insert(name, value);
                }

                OpCode::DefineGlobal => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    let value = self.pop()?;
                    self.globals.insert(name, value);
                }

                OpCode::GetUpvalue => {
                    let idx = self.read_byte() as usize;
                    let upvalue = {
                        let frame = self.current_frame();
                        frame.closure.borrow().upvalues[idx].clone()
                    };
                    let value = match &*upvalue.borrow() {
                        Upvalue::Open(slot) => self.stack[*slot].clone(),
                        Upvalue::Closed(val) => val.clone(),
                    };
                    self.push(value);
                }

                OpCode::SetUpvalue => {
                    let idx = self.read_byte() as usize;
                    let value = self.peek(0)?.clone();
                    let upvalue = {
                        let frame = self.current_frame();
                        frame.closure.borrow().upvalues[idx].clone()
                    };
                    // Get the slot index if open, then drop the borrow before mutating stack
                    let slot_to_update = {
                        let borrowed = upvalue.borrow();
                        if let Upvalue::Open(slot) = &*borrowed {
                            Some(*slot)
                        } else {
                            None
                        }
                    };
                    if let Some(slot) = slot_to_update {
                        self.stack[slot] = value;
                    } else {
                        // Must be Closed
                        if let Upvalue::Closed(val) = &mut *upvalue.borrow_mut() {
                            *val = value;
                        }
                    }
                }

                OpCode::CloseUpvalue => {
                    let slot = self.stack.len() - 1;
                    self.close_upvalues(slot);
                    self.pop()?;
                }

                OpCode::Add => self.binary_op(|a, b| match (a, b) {
                    (VMValue::Int(x), VMValue::Int(y)) => Ok(VMValue::Int(x + y)),
                    (VMValue::Float(x), VMValue::Float(y)) => Ok(VMValue::Float(x + y)),
                    (VMValue::Int(x), VMValue::Float(y)) => Ok(VMValue::Float(x as f64 + y)),
                    (VMValue::Float(x), VMValue::Int(y)) => Ok(VMValue::Float(x + y as f64)),
                    (VMValue::String(x), VMValue::String(y)) => {
                        Ok(VMValue::String(Rc::new(format!("{}{}", x, y))))
                    }
                    _ => Err(RuntimeError::new("Invalid operands for +", Span::default())),
                })?,

                OpCode::Subtract => self.binary_op(|a, b| match (a, b) {
                    (VMValue::Int(x), VMValue::Int(y)) => Ok(VMValue::Int(x - y)),
                    (VMValue::Float(x), VMValue::Float(y)) => Ok(VMValue::Float(x - y)),
                    (VMValue::Int(x), VMValue::Float(y)) => Ok(VMValue::Float(x as f64 - y)),
                    (VMValue::Float(x), VMValue::Int(y)) => Ok(VMValue::Float(x - y as f64)),
                    _ => Err(RuntimeError::new("Invalid operands for -", Span::default())),
                })?,

                OpCode::Multiply => self.binary_op(|a, b| match (a, b) {
                    (VMValue::Int(x), VMValue::Int(y)) => Ok(VMValue::Int(x * y)),
                    (VMValue::Float(x), VMValue::Float(y)) => Ok(VMValue::Float(x * y)),
                    (VMValue::Int(x), VMValue::Float(y)) => Ok(VMValue::Float(x as f64 * y)),
                    (VMValue::Float(x), VMValue::Int(y)) => Ok(VMValue::Float(x * y as f64)),
                    _ => Err(RuntimeError::new("Invalid operands for *", Span::default())),
                })?,

                OpCode::Divide => self.binary_op(|a, b| match (a, b) {
                    (VMValue::Int(x), VMValue::Int(y)) => {
                        if y == 0 {
                            Err(RuntimeError::new("Division by zero", Span::default()))
                        } else {
                            Ok(VMValue::Int(x / y))
                        }
                    }
                    (VMValue::Float(x), VMValue::Float(y)) => Ok(VMValue::Float(x / y)),
                    (VMValue::Int(x), VMValue::Float(y)) => Ok(VMValue::Float(x as f64 / y)),
                    (VMValue::Float(x), VMValue::Int(y)) => Ok(VMValue::Float(x / y as f64)),
                    _ => Err(RuntimeError::new("Invalid operands for /", Span::default())),
                })?,

                OpCode::Modulo => self.binary_op(|a, b| match (a, b) {
                    (VMValue::Int(x), VMValue::Int(y)) => {
                        if y == 0 {
                            Err(RuntimeError::new("Modulo by zero", Span::default()))
                        } else {
                            Ok(VMValue::Int(x % y))
                        }
                    }
                    (VMValue::Float(x), VMValue::Float(y)) => Ok(VMValue::Float(x % y)),
                    (VMValue::Int(x), VMValue::Float(y)) => Ok(VMValue::Float(x as f64 % y)),
                    (VMValue::Float(x), VMValue::Int(y)) => Ok(VMValue::Float(x % y as f64)),
                    _ => Err(RuntimeError::new("Invalid operands for %", Span::default())),
                })?,

                OpCode::Negate => {
                    let value = self.pop()?;
                    let result = match value {
                        VMValue::Int(n) => VMValue::Int(-n),
                        VMValue::Float(n) => VMValue::Float(-n),
                        _ => {
                            return Err(RuntimeError::new(
                                format!("Cannot negate {}", value.type_name()),
                                Span::default(),
                            ))
                        }
                    };
                    self.push(result);
                }

                OpCode::Equal => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(VMValue::Bool(a == b));
                }

                OpCode::NotEqual => {
                    let b = self.pop()?;
                    let a = self.pop()?;
                    self.push(VMValue::Bool(a != b));
                }

                OpCode::Less => self.comparison_op(|a, b| a < b)?,
                OpCode::LessEqual => self.comparison_op(|a, b| a <= b)?,
                OpCode::Greater => self.comparison_op(|a, b| a > b)?,
                OpCode::GreaterEqual => self.comparison_op(|a, b| a >= b)?,

                OpCode::Range => {
                    let end = self.pop()?;
                    let start = self.pop()?;
                    match (&start, &end) {
                        (VMValue::Int(s), VMValue::Int(e)) => {
                            let arr: Vec<VMValue> = (*s..*e).map(VMValue::Int).collect();
                            self.push(VMValue::Array(Rc::new(RefCell::new(arr))));
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!(
                                    "range (..) expects two integers, got {} and {}",
                                    start.type_name(),
                                    end.type_name()
                                ),
                                Span::default(),
                            ));
                        }
                    }
                }

                OpCode::Not => {
                    let value = self.pop()?;
                    self.push(VMValue::Bool(!value.is_truthy()));
                }

                OpCode::Jump => {
                    let offset = self.read_u16() as usize;
                    self.current_frame_mut().ip += offset;
                }

                OpCode::JumpIfFalse => {
                    let offset = self.read_u16() as usize;
                    let condition = self.peek(0)?;
                    if !condition.is_truthy() {
                        self.current_frame_mut().ip += offset;
                    }
                }

                OpCode::JumpIfTrue => {
                    let offset = self.read_u16() as usize;
                    let condition = self.peek(0)?;
                    if condition.is_truthy() {
                        self.current_frame_mut().ip += offset;
                    }
                }

                OpCode::JumpIfFalseNoPop => {
                    let offset = self.read_u16() as usize;
                    let condition = self.peek(0)?;
                    if !condition.is_truthy() {
                        self.current_frame_mut().ip += offset;
                    }
                }

                OpCode::JumpIfTrueNoPop => {
                    let offset = self.read_u16() as usize;
                    let condition = self.peek(0)?;
                    if condition.is_truthy() {
                        self.current_frame_mut().ip += offset;
                    }
                }

                OpCode::Loop => {
                    let offset = self.read_u16() as usize;
                    self.current_frame_mut().ip -= offset;
                }

                OpCode::Call => {
                    let arg_count = self.read_byte() as usize;
                    self.call_value(arg_count)?;
                }

                OpCode::Invoke => {
                    let name_idx = self.read_u16();
                    let arg_count = self.read_byte() as usize;
                    let name = self.read_string_constant(name_idx)?;
                    self.invoke(&name, arg_count)?;
                }

                OpCode::SuperInvoke => {
                    let _name_idx = self.read_u16();
                    let _arg_count = self.read_byte() as usize;
                    // Super invoke implementation would go here
                }

                OpCode::Return => {
                    let result = self.pop()?;
                    let frame = self.frames.pop().unwrap();

                    // Close any remaining upvalues
                    self.close_upvalues(frame.slots_start);

                    // Pop all locals
                    self.stack.truncate(frame.slots_start);

                    if self.frames.is_empty() {
                        return Ok(());
                    }

                    self.push(result);
                }

                OpCode::Closure => {
                    let func_idx = self.read_u16();
                    let function = self.read_function_constant(func_idx)?;
                    let upvalue_count = function.upvalue_count;

                    let mut closure = Closure::new(function);

                    for _ in 0..upvalue_count {
                        let is_local = self.read_byte() != 0;
                        let index = self.read_byte() as usize;

                        let upvalue = if is_local {
                            let slots_start = self.current_frame().slots_start;
                            self.capture_upvalue(slots_start + index)
                        } else {
                            let frame = self.current_frame();
                            frame.closure.borrow().upvalues[index].clone()
                        };

                        closure.upvalues.push(upvalue);
                    }

                    self.push(VMValue::Closure(Rc::new(RefCell::new(closure))));
                }

                OpCode::Class => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    let class = VMClass::new(name);
                    self.push(VMValue::Class(Rc::new(RefCell::new(class))));
                }

                OpCode::Inherit => {
                    let superclass = self.peek(1)?.clone();
                    if let VMValue::Class(superclass) = superclass {
                        if let VMValue::Class(subclass) = self.peek(0)?.clone() {
                            subclass.borrow_mut().superclass = Some(superclass.clone());
                            // Copy methods from superclass
                            let methods = superclass.borrow().methods.clone();
                            subclass.borrow_mut().methods = methods;
                        }
                    } else {
                        return Err(RuntimeError::new(
                            "Superclass must be a class",
                            Span::default(),
                        ));
                    }
                    self.pop()?; // Pop superclass
                }

                OpCode::Method => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    self.define_method(&name, false)?;
                }

                OpCode::StaticMethod => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    self.define_method(&name, true)?;
                }

                OpCode::GetProperty => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    let object = self.pop()?;

                    match object {
                        VMValue::Instance(inst) => {
                            // Check for field first
                            let field_value = inst.borrow().get(&name);
                            if let Some(value) = field_value {
                                self.push(value);
                            } else {
                                // Check for method
                                let class = inst.borrow().class.clone();
                                let class_name = class.borrow().name.clone();

                                // Check if this is a known DateTime/Duration instance method
                                if class_name == "DateTime" || class_name == "Duration" {
                                    let method_names = [
                                        "year",
                                        "month",
                                        "day",
                                        "hour",
                                        "minute",
                                        "second",
                                        "weekday",
                                        "to_unix",
                                        "to_iso",
                                        "to_string",
                                        "str",
                                        "total_seconds",
                                        "total_minutes",
                                        "total_hours",
                                        "total_days",
                                        "total_weeks",
                                    ];
                                    if method_names.contains(&name.as_str()) {
                                        let bound = VMValue::BoundNativeMethod(
                                            inst.clone(),
                                            class_name,
                                            name,
                                        );
                                        self.push(bound);
                                    } else {
                                        return Err(RuntimeError::new(
                                            format!("Undefined property '{}'", name),
                                            Span::default(),
                                        ));
                                    }
                                } else {
                                    // Check for regular method
                                    let method = class.borrow().find_method(&name);
                                    if let Some(method) = method {
                                        let bound = VMValue::BoundMethod(inst.clone(), method);
                                        self.push(bound);
                                    } else if class.borrow().find_native_method(&name).is_some() {
                                        // Check for native method
                                        let class_name = class.borrow().name.clone();
                                        let bound = VMValue::BoundNativeMethod(
                                            inst.clone(),
                                            class_name,
                                            name,
                                        );
                                        self.push(bound);
                                    } else {
                                        return Err(RuntimeError::new(
                                            format!("Undefined property '{}'", name),
                                            Span::default(),
                                        ));
                                    }
                                }
                            }
                        }
                        VMValue::Class(class) => {
                            // Static method access
                            if let Some(method) = class.borrow().static_methods.get(&name) {
                                self.push(VMValue::Closure(method.clone()));
                            } else {
                                // Check for built-in DateTime/Duration static methods
                                let class_name = class.borrow().name.clone();
                                if class_name == "DateTime" {
                                    match name.as_str() {
                                        "utc" | "now" => {
                                            // Return a closure that creates a DateTime instance
                                            let closure = create_datetime_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        "parse" => {
                                            // Return a closure that parses a datetime string
                                            let closure = create_datetime_parse_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                format!("Undefined static method '{}'", name),
                                                Span::default(),
                                            ));
                                        }
                                    }
                                } else if class_name == "Duration" {
                                    match name.as_str() {
                                        "between" => {
                                            let closure = create_duration_between_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        "seconds" | "minutes" | "hours" | "days" | "weeks" => {
                                            let closure = create_duration_from_value_closure(name);
                                            self.push(VMValue::Closure(closure));
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                format!("Undefined static method '{}'", name),
                                                Span::default(),
                                            ));
                                        }
                                    }
                                } else if class_name == "I18n" {
                                    match name.as_str() {
                                        "locale" => {
                                            let closure = create_i18n_locale_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        "set_locale" => {
                                            let closure = create_i18n_set_locale_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        "translate" => {
                                            let closure = create_i18n_translate_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        "plural" => {
                                            let closure = create_i18n_plural_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        "format_number" => {
                                            let closure = create_i18n_format_number_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        "format_currency" => {
                                            let closure = create_i18n_format_currency_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        "format_date" => {
                                            let closure = create_i18n_format_date_closure();
                                            self.push(VMValue::Closure(closure));
                                        }
                                        _ => {
                                            return Err(RuntimeError::new(
                                                format!("Undefined static method '{}'", name),
                                                Span::default(),
                                            ));
                                        }
                                    }
                                } else {
                                    return Err(RuntimeError::new(
                                        format!("Undefined static method '{}'", name),
                                        Span::default(),
                                    ));
                                }
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!("Cannot access property on {}", object.type_name()),
                                Span::default(),
                            ))
                        }
                    }
                }

                OpCode::SetProperty => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    let value = self.pop()?;
                    let object = self.pop()?;

                    if let VMValue::Instance(inst) = object {
                        inst.borrow_mut().set(name, value.clone());
                        self.push(value);
                    } else {
                        return Err(RuntimeError::new(
                            format!("Cannot set property on {}", object.type_name()),
                            Span::default(),
                        ));
                    }
                }

                OpCode::GetThis => {
                    // 'this' is at local slot 0, which is slots_start + 1 (slot 0 is the closure)
                    let slots_start = self.current_frame().slots_start;
                    let value = self.stack[slots_start + 1].clone();
                    self.push(value);
                }

                OpCode::GetSuper => {
                    // Get the superclass from 'this' (local slot 0 = slots_start + 1)
                    let slots_start = self.current_frame().slots_start;
                    // Extract superclass value before pushing (to avoid borrow conflict)
                    let superclass_value = {
                        if let VMValue::Instance(inst) = &self.stack[slots_start + 1] {
                            inst.borrow().class.borrow().superclass.clone()
                        } else {
                            return Err(RuntimeError::new(
                                "'super' used outside of instance",
                                Span::default(),
                            ));
                        }
                    };
                    if let Some(superclass) = superclass_value {
                        self.push(VMValue::Class(superclass));
                    } else {
                        return Err(RuntimeError::new("No superclass", Span::default()));
                    }
                }

                OpCode::New => {
                    let name_idx = self.read_u16();
                    let arg_count = self.read_byte() as usize;
                    let name = self.read_string_constant(name_idx)?;

                    // Get the class
                    let class = self.globals.get(&name).cloned().ok_or_else(|| {
                        RuntimeError::new(format!("Undefined class '{}'", name), Span::default())
                    })?;

                    if let VMValue::Class(class) = class {
                        // Create instance
                        let instance = VMInstance::new(class.clone());
                        let instance = Rc::new(RefCell::new(instance));

                        // Call constructor if present
                        if let Some(constructor) = class.borrow().constructor.clone() {
                            // Push instance as 'this'
                            let instance_value = VMValue::Instance(instance.clone());

                            // Set up for constructor call
                            let args_start = self.stack.len() - arg_count;
                            self.stack.insert(args_start, instance_value.clone());

                            // Call the constructor
                            self.call_closure_frame(constructor, arg_count + 1)?;
                        } else {
                            // No constructor, just push the instance
                            // But we need to pop the arguments
                            for _ in 0..arg_count {
                                self.pop()?;
                            }
                            self.push(VMValue::Instance(instance));
                        }
                    } else {
                        return Err(RuntimeError::new(
                            format!("'{}' is not a class", name),
                            Span::default(),
                        ));
                    }
                }

                OpCode::BuildArray => {
                    let count = self.read_u16() as usize;
                    let mut elements = Vec::with_capacity(count);

                    for _ in 0..count {
                        elements.push(self.pop()?);
                    }
                    elements.reverse();

                    self.push(VMValue::Array(Rc::new(RefCell::new(elements))));
                }

                OpCode::BuildHash => {
                    let pair_count = self.read_u16() as usize;
                    let mut pairs = Vec::with_capacity(pair_count);

                    for _ in 0..pair_count {
                        let value = self.pop()?;
                        let key = self.pop()?;
                        pairs.push((key, value));
                    }
                    pairs.reverse();

                    self.push(VMValue::Hash(Rc::new(RefCell::new(pairs))));
                }

                OpCode::Index => {
                    let index = self.pop()?;
                    let object = self.pop()?;

                    let value = match (&object, &index) {
                        (VMValue::Array(arr), VMValue::Int(i)) => {
                            let arr = arr.borrow();
                            let idx = if *i < 0 {
                                (arr.len() as i64 + *i) as usize
                            } else {
                                *i as usize
                            };
                            arr.get(idx).cloned().unwrap_or(VMValue::Null)
                        }
                        (VMValue::String(s), VMValue::Int(i)) => {
                            let chars: Vec<char> = s.chars().collect();
                            let idx = if *i < 0 {
                                (chars.len() as i64 + *i) as usize
                            } else {
                                *i as usize
                            };
                            chars
                                .get(idx)
                                .map(|c| VMValue::String(Rc::new(c.to_string())))
                                .unwrap_or(VMValue::Null)
                        }
                        (VMValue::Hash(hash), key) => {
                            let hash = hash.borrow();
                            hash.iter()
                                .find(|(k, _)| k.hash_eq(key))
                                .map(|(_, v)| v.clone())
                                .unwrap_or(VMValue::Null)
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!(
                                    "Cannot index {} with {}",
                                    object.type_name(),
                                    index.type_name()
                                ),
                                Span::default(),
                            ))
                        }
                    };

                    self.push(value);
                }

                OpCode::IndexSet => {
                    let value = self.pop()?;
                    let index = self.pop()?;
                    let object = self.pop()?;

                    match (&object, &index) {
                        (VMValue::Array(arr), VMValue::Int(i)) => {
                            let mut arr = arr.borrow_mut();
                            let idx = if *i < 0 {
                                (arr.len() as i64 + *i) as usize
                            } else {
                                *i as usize
                            };
                            if idx < arr.len() {
                                arr[idx] = value.clone();
                            } else {
                                return Err(RuntimeError::new(
                                    format!("Index {} out of bounds", i),
                                    Span::default(),
                                ));
                            }
                        }
                        (VMValue::Hash(hash), key) => {
                            let mut hash = hash.borrow_mut();
                            // Update existing or insert new
                            let mut found = false;
                            for (k, v) in hash.iter_mut() {
                                if k.hash_eq(key) {
                                    *v = value.clone();
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                hash.push((index, value.clone()));
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!(
                                    "Cannot index {} with {}",
                                    object.type_name(),
                                    index.type_name()
                                ),
                                Span::default(),
                            ))
                        }
                    }

                    self.push(value);
                }

                OpCode::GetIterator => {
                    let iterable = self.pop()?;
                    let iterator = match iterable {
                        VMValue::Array(arr) => VMIterator::Array {
                            array: arr,
                            index: 0,
                        },
                        VMValue::Hash(hash) => VMIterator::Hash {
                            pairs: hash.borrow().clone(),
                            index: 0,
                        },
                        VMValue::String(s) => VMIterator::String {
                            chars: s.chars().collect(),
                            index: 0,
                        },
                        _ => {
                            return Err(RuntimeError::new(
                                format!("Cannot iterate over {}", iterable.type_name()),
                                Span::default(),
                            ))
                        }
                    };
                    self.push(VMValue::Iterator(Rc::new(RefCell::new(iterator))));
                }

                OpCode::IteratorNext => {
                    let jump_offset = self.read_u16() as usize;

                    // Peek at the iterator (don't pop it)
                    let iterator = self.peek(0)?.clone();
                    if let VMValue::Iterator(iter) = iterator {
                        if let Some(value) = iter.borrow_mut().next() {
                            self.push(value);
                        } else {
                            // Iterator exhausted, jump
                            self.pop()?; // Pop the iterator
                            self.current_frame_mut().ip += jump_offset;
                        }
                    } else {
                        return Err(RuntimeError::new("Expected iterator", Span::default()));
                    }
                }

                OpCode::NativeCall => {
                    let native_idx = self.read_u16();
                    let arg_count = self.read_byte() as usize;

                    let native_id = NativeId::from_u16(native_idx).ok_or_else(|| {
                        RuntimeError::new(
                            format!("Invalid native function: {}", native_idx),
                            Span::default(),
                        )
                    })?;

                    // Check arity
                    if let Some(expected_arity) = native_id.arity() {
                        if arg_count != expected_arity {
                            return Err(RuntimeError::new(
                                format!(
                                    "Expected {} arguments but got {}",
                                    expected_arity, arg_count
                                ),
                                Span::default(),
                            ));
                        }
                    }

                    // Collect arguments
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(self.pop()?);
                    }
                    args.reverse();

                    // Call the native function
                    let result = self.call_native(native_id, args)?;
                    self.push(result);
                }

                OpCode::Print => {
                    let value = self.peek(0)?;
                    println!("{}", value);
                }

                OpCode::LoadDefault => {
                    // This opcode is handled in call_closure_frame, not here
                    // It's used to indicate where default values are stored
                    // The operand is the constant index
                    let _default_idx = self.read_u16();
                    // Default values are loaded by call_closure_frame before pushing the frame
                }

                OpCode::SpreadArray => {
                    // Pop an array from the stack and push its elements
                    let array_val = self.pop()?;
                    match array_val {
                        VMValue::Array(arr) => {
                            let arr = arr.borrow();
                            for val in arr.iter() {
                                self.push(val.clone());
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "cannot spread non-array",
                                Span::default(),
                            ));
                        }
                    }
                }

                OpCode::SpreadHash => {
                    // Pop a hash from the stack and push its key-value pairs
                    let hash_val = self.pop()?;
                    match hash_val {
                        VMValue::Hash(hash) => {
                            let hash = hash.borrow();
                            for (key, val) in hash.iter() {
                                self.push(key.clone());
                                self.push(val.clone());
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "cannot spread non-hash",
                                Span::default(),
                            ));
                        }
                    }
                }

                OpCode::TypeCheck => {
                    let type_idx = self.read_u16();
                    let type_name = self.read_string_constant(type_idx)?;
                    let value = self.pop()?;

                    let matches = match &value {
                        VMValue::Int(_) => type_name == "Int",
                        VMValue::Float(_) => type_name == "Float",
                        VMValue::Bool(_) => type_name == "Bool",
                        VMValue::String(_) => type_name == "String",
                        VMValue::Null => type_name == "Void",
                        VMValue::Instance(inst) => inst.borrow().class.borrow().name == type_name,
                        VMValue::Array(_) => type_name == "Array",
                        VMValue::Hash(_) => type_name == "Hash",
                        _ => false,
                    };

                    self.push(VMValue::Bool(matches));
                }

                OpCode::ArrayLen => {
                    let array_val = self.pop()?;
                    match array_val {
                        VMValue::Array(arr) => {
                            let len = arr.borrow().len() as i64;
                            self.push(VMValue::Int(len));
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "array_len requires an array",
                                Span::default(),
                            ));
                        }
                    }
                }

                OpCode::GetPropertyStr => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    let object = self.pop()?;

                    match object {
                        VMValue::Instance(inst) => {
                            let field_value = inst.borrow().get(&name);
                            self.push(field_value.unwrap_or(VMValue::Null));
                        }
                        VMValue::Hash(hash) => {
                            let hash = hash.borrow();
                            let key = VMValue::String(Rc::new(name));
                            let val = hash.iter().find(|(k, _)| k == &key);
                            match val {
                                Some((_, v)) => self.push(v.clone()),
                                None => self.push(VMValue::Null),
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!("Cannot get property '{}' on {}", name, object.type_name()),
                                Span::default(),
                            ));
                        }
                    }
                }

                OpCode::GetFieldStr => {
                    let name_idx = self.read_u16();
                    let name = self.read_string_constant(name_idx)?;
                    let instance = self.pop()?;

                    match instance {
                        VMValue::Instance(inst) => {
                            let field_value = inst.borrow().get(&name);
                            match field_value {
                                Some(v) => self.push(v.clone()),
                                None => {
                                    return Err(RuntimeError::new(
                                        format!("Field '{}' not found", name),
                                        Span::default(),
                                    ));
                                }
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                format!("Cannot get field '{}' on non-instance", name),
                                Span::default(),
                            ));
                        }
                    }
                }

                OpCode::BuildArrayFromStack => {
                    let count = self.read_u16() as usize;
                    let mut elements = Vec::with_capacity(count);
                    for _ in 0..count {
                        elements.push(self.pop()?);
                    }
                    elements.reverse();
                    self.push(VMValue::Array(Rc::new(RefCell::new(elements))));
                }

                OpCode::BuildHashFromStack => {
                    let pair_count = self.read_u16() as usize;
                    let mut pairs = Vec::with_capacity(pair_count);
                    for _ in 0..pair_count {
                        let value = self.pop()?;
                        let key = self.pop()?;
                        pairs.push((key, value));
                    }
                    self.push(VMValue::Hash(Rc::new(RefCell::new(pairs))));
                }

                OpCode::StoreBinding => {
                    let _name_idx = self.read_u16();
                    // Binding storage is handled at a higher level
                    // This opcode would need access to a bindings map
                    // For now, this is a placeholder
                }

                // ============ Exception Handling ============
                OpCode::Try => {
                    let catch_offset = self.read_u16() as usize;
                    let finally_offset = self.read_u16() as usize;

                    let handler = ExceptionHandler {
                        catch_ip: catch_offset,
                        finally_ip: finally_offset,
                        stack_depth: self.stack.len(),
                        local_slots: 0,
                    };
                    self.exception_handlers.push(handler);
                }

                OpCode::TryEnd => {
                    // Normal exit from try block - pop the handler
                    // But don't pop if there's a finally (finally will do it)
                    if let Some(handler) = self.exception_handlers.last() {
                        if handler.finally_ip == 0 {
                            self.exception_handlers.pop();
                        }
                    }
                }

                OpCode::Throw => {
                    let error = self.pop()?;
                    self.throw_exception(error)?;
                }

                OpCode::Rethrow => {
                    // Re-throw the current exception
                    if let Some(ref error) = self.current_exception {
                        self.throw_exception(error.clone())?;
                    } else {
                        return Err(RuntimeError::new(
                            "No exception to rethrow",
                            Span::default(),
                        ));
                    }
                }

                OpCode::PopTry => {
                    // Pop the exception handler (called after catch/finally)
                    self.exception_handlers.pop();
                }
            }
        }
    }

    /// Throw an exception and find the appropriate handler.
    fn throw_exception(&mut self, error: VMValue) -> VMResult<()> {
        self.current_exception = Some(error.clone());

        // Unwind stack until we find a handler
        while let Some(handler) = self.exception_handlers.pop() {
            // Check if we have a catch handler
            if handler.catch_ip > 0 {
                // Unwind to handler's stack depth
                self.unwind_stack(handler.stack_depth);

                // Push the exception value
                self.push(error);

                // Jump to catch block
                self.current_frame_mut().ip = handler.catch_ip;

                return Ok(());
            }
            // Check if we have a finally handler
            else if handler.finally_ip > 0 {
                // Unwind to handler's stack depth
                self.unwind_stack(handler.stack_depth);

                // Push the exception value (for potential rethrow in finally)
                self.push(error.clone());

                // Jump to finally block
                self.current_frame_mut().ip = handler.finally_ip;

                return Ok(());
            }
            // No catch or finally, continue unwinding
        }

        // No handler found - propagate the error
        Err(RuntimeError::new(
            format!("Unhandled exception: {}", error),
            Span::default(),
        ))
    }

    /// Unwind the stack to the given depth, closing upvalues.
    fn unwind_stack(&mut self, target_depth: usize) {
        // Close upvalues
        while self.stack.len() > target_depth {
            let slot = self.stack.len() - 1;
            self.close_upvalues(slot);
            self.stack.pop();
        }
    }

    // ===== Helper methods =====

    fn current_frame(&self) -> &CallFrame {
        self.frames.last().expect("No call frame")
    }

    fn current_frame_mut(&mut self) -> &mut CallFrame {
        self.frames.last_mut().expect("No call frame")
    }

    fn read_byte(&mut self) -> u8 {
        let frame = self.current_frame();
        let byte = frame.closure.borrow().function.chunk.code[frame.ip];
        self.current_frame_mut().ip += 1;
        byte
    }

    fn read_u16(&mut self) -> u16 {
        let frame = self.current_frame();
        let value = frame.closure.borrow().function.chunk.read_u16(frame.ip);
        self.current_frame_mut().ip += 2;
        value
    }

    fn read_constant(&self, index: u16) -> VMResult<VMValue> {
        let frame = self.current_frame();
        let constant = frame
            .closure
            .borrow()
            .function
            .chunk
            .constants
            .get(index as usize)
            .ok_or_else(|| RuntimeError::new("Invalid constant index", Span::default()))?
            .clone();

        Ok(match constant {
            Constant::Int(n) => VMValue::Int(n),
            Constant::Float(n) => VMValue::Float(n),
            Constant::String(s) => VMValue::String(Rc::new(s)),
            Constant::Function(f) => VMValue::Closure(Rc::new(RefCell::new(Closure::new(f)))),
            Constant::Class(c) => {
                let class = VMClass::new(c.name.clone());
                VMValue::Class(Rc::new(RefCell::new(class)))
            }
            Constant::Null => VMValue::Null,
        })
    }

    /// Read a constant from a specific function's constant pool
    fn read_constant_from_function(function: &CompiledFunction, index: u16) -> VMResult<VMValue> {
        let constant = function
            .chunk
            .constants
            .get(index as usize)
            .ok_or_else(|| RuntimeError::new("Invalid constant index", Span::default()))?
            .clone();

        Ok(match constant {
            Constant::Int(n) => VMValue::Int(n),
            Constant::Float(n) => VMValue::Float(n),
            Constant::String(s) => VMValue::String(Rc::new(s)),
            Constant::Function(f) => VMValue::Closure(Rc::new(RefCell::new(Closure::new(f)))),
            Constant::Class(c) => {
                let class = VMClass::new(c.name.clone());
                VMValue::Class(Rc::new(RefCell::new(class)))
            }
            Constant::Null => VMValue::Null,
        })
    }

    fn read_string_constant(&self, index: u16) -> VMResult<String> {
        let frame = self.current_frame();
        let constant = frame
            .closure
            .borrow()
            .function
            .chunk
            .constants
            .get(index as usize)
            .ok_or_else(|| RuntimeError::new("Invalid constant index", Span::default()))?
            .clone();

        match constant {
            Constant::String(s) => Ok(s),
            _ => Err(RuntimeError::new(
                "Expected string constant",
                Span::default(),
            )),
        }
    }

    fn read_function_constant(&self, index: u16) -> VMResult<Rc<CompiledFunction>> {
        let frame = self.current_frame();
        let constant = frame
            .closure
            .borrow()
            .function
            .chunk
            .constants
            .get(index as usize)
            .ok_or_else(|| RuntimeError::new("Invalid constant index", Span::default()))?
            .clone();

        match constant {
            Constant::Function(f) => Ok(f),
            _ => Err(RuntimeError::new(
                "Expected function constant",
                Span::default(),
            )),
        }
    }

    fn push(&mut self, value: VMValue) {
        self.stack.push(value);
    }

    fn pop(&mut self) -> VMResult<VMValue> {
        self.stack
            .pop()
            .ok_or_else(|| RuntimeError::new("Stack underflow", Span::default()))
    }

    fn peek(&self, distance: usize) -> VMResult<&VMValue> {
        let index = self.stack.len().saturating_sub(1 + distance);
        self.stack
            .get(index)
            .ok_or_else(|| RuntimeError::new("Stack underflow", Span::default()))
    }

    fn binary_op<F>(&mut self, op: F) -> VMResult<()>
    where
        F: FnOnce(VMValue, VMValue) -> VMResult<VMValue>,
    {
        let b = self.pop()?;
        let a = self.pop()?;
        let result = op(a, b)?;
        self.push(result);
        Ok(())
    }

    fn comparison_op<F>(&mut self, op: F) -> VMResult<()>
    where
        F: FnOnce(f64, f64) -> bool,
    {
        let b = self.pop()?;
        let a = self.pop()?;

        let result = match (&a, &b) {
            (VMValue::Int(x), VMValue::Int(y)) => op(*x as f64, *y as f64),
            (VMValue::Float(x), VMValue::Float(y)) => op(*x, *y),
            (VMValue::Int(x), VMValue::Float(y)) => op(*x as f64, *y),
            (VMValue::Float(x), VMValue::Int(y)) => op(*x, *y as f64),
            (VMValue::String(x), VMValue::String(y)) => {
                let cmp = x.cmp(y);
                match cmp {
                    std::cmp::Ordering::Less => op(-1.0, 0.0),
                    std::cmp::Ordering::Equal => op(0.0, 0.0),
                    std::cmp::Ordering::Greater => op(1.0, 0.0),
                }
            }
            _ => {
                return Err(RuntimeError::new(
                    format!("Cannot compare {} and {}", a.type_name(), b.type_name()),
                    Span::default(),
                ))
            }
        };

        self.push(VMValue::Bool(result));
        Ok(())
    }

    fn call_value(&mut self, arg_count: usize) -> VMResult<()> {
        let callee = self.peek(arg_count)?.clone();

        match callee {
            VMValue::Closure(closure) => {
                self.call_closure_frame(closure, arg_count)?;
            }
            VMValue::BoundMethod(instance, method) => {
                // Replace the callee (bound method) with 'this' (the instance)
                let callee_idx = self.stack.len() - arg_count - 1;
                self.stack[callee_idx] = VMValue::Instance(instance);
                self.call_closure_frame(method, arg_count + 1)?;
            }
            VMValue::BoundNativeMethod(instance, _class_name, method_name) => {
                // Call the native method directly
                let inst = instance;
                let result = call_datetime_method(&inst.borrow(), &method_name)
                    .map_err(|e| RuntimeError::new(e, Span::default()))?;
                // Pop the bound method
                self.pop()?;
                // Push result
                self.push(result);
            }
            VMValue::Class(class) => {
                // Create instance
                let instance = VMInstance::new(class.clone());
                let instance = Rc::new(RefCell::new(instance));

                // Replace class with instance on stack
                let callee_idx = self.stack.len() - arg_count - 1;
                self.stack[callee_idx] = VMValue::Instance(instance.clone());

                // Call constructor if present
                if let Some(constructor) = class.borrow().constructor.clone() {
                    self.call_closure_frame(constructor, arg_count + 1)?;
                }
            }
            VMValue::NativeFunction(idx) => {
                let native_id = NativeId::from_u16(idx)
                    .ok_or_else(|| RuntimeError::new("Invalid native function", Span::default()))?;
                // Collect arguments
                let mut args = Vec::with_capacity(arg_count);
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();
                self.pop()?; // Pop the native function itself

                // Call
                let result = self.call_native(native_id, args)?;
                self.push(result);
            }
            _ => {
                return Err(RuntimeError::new(
                    format!("Cannot call {}", callee.type_name()),
                    Span::default(),
                ))
            }
        }

        Ok(())
    }

    fn call_closure_frame(
        &mut self,
        closure: Rc<RefCell<Closure>>,
        arg_count: usize,
    ) -> VMResult<()> {
        let function = closure.borrow().function.clone();
        let function_name = function.name.clone();

        // Handle built-in static method closures directly
        if function_name == "datetime_utc" {
            return self.handle_datetime_utc(arg_count);
        } else if function_name == "datetime_parse" {
            return self.handle_datetime_parse(arg_count);
        } else if function_name == "duration_between" {
            return self.handle_duration_between(arg_count);
        } else if function_name.starts_with("duration_") {
            let method_name = function_name.strip_prefix("duration_").unwrap();
            return self.handle_duration_from_value(method_name, arg_count);
        } else if function_name.starts_with("i18n_") {
            return self.handle_i18n_method(&function_name, arg_count);
        }

        let arity = function.arity as usize;
        let full_arity = function.full_arity as usize;

        // Check if we have enough arguments
        if arg_count < arity {
            return Err(RuntimeError::new(
                format!(
                    "Expected at least {} arguments but got {}",
                    arity, arg_count
                ),
                Span::default(),
            ));
        }

        // Check if we have too many arguments
        if arg_count > full_arity {
            return Err(RuntimeError::new(
                format!(
                    "Expected at most {} arguments but got {}",
                    full_arity, arg_count
                ),
                Span::default(),
            ));
        }

        // Fill in default values for missing arguments
        let defaults_needed = full_arity - arg_count;
        if defaults_needed > 0 {
            // Load default values from the function's constant pool
            // Default values are stored as the last N constants (in reverse order)
            let default_start_idx = function.chunk.constants.len() - defaults_needed;
            for i in default_start_idx..function.chunk.constants.len() {
                let default_val = Self::read_constant_from_function(&function, i as u16)?;
                self.push(default_val);
            }
        }

        let final_arg_count = full_arity;

        if self.frames.len() >= FRAMES_MAX {
            return Err(RuntimeError::new("Stack overflow", Span::default()));
        }

        // slots_start includes the closure itself, which will be replaced by the return value
        let slots_start = self.stack.len() - final_arg_count - 1;

        self.frames.push(CallFrame {
            closure,
            ip: 0,
            slots_start,
        });

        Ok(())
    }

    /// Handle datetime_utc() static method call
    fn handle_datetime_utc(&mut self, _arg_count: usize) -> VMResult<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let timestamp = now.as_secs() as i64;
        let class = Rc::new(RefCell::new(VMClass::new("DateTime".to_string())));
        let instance =
            VMInstance::with_field(class, "timestamp".to_string(), VMValue::Int(timestamp));
        // Pop the closure
        self.pop()?;
        // Push result
        self.push(VMValue::Instance(Rc::new(RefCell::new(instance))));
        Ok(())
    }

    /// Handle datetime_parse(string) static method call
    fn handle_datetime_parse(&mut self, arg_count: usize) -> VMResult<()> {
        if arg_count != 1 {
            return Err(RuntimeError::new(
                "datetime_parse expects 1 argument",
                Span::default(),
            ));
        }
        let arg = self.pop()?;
        let s = match arg {
            VMValue::String(s) => s.as_str().to_string(),
            _ => {
                return Err(RuntimeError::new(
                    "datetime_parse requires a string",
                    Span::default(),
                ))
            }
        };
        let timestamp =
            parse_datetime_string(&s).map_err(|e| RuntimeError::new(e, Span::default()))?;
        let class = Rc::new(RefCell::new(VMClass::new("DateTime".to_string())));
        let instance =
            VMInstance::with_field(class, "timestamp".to_string(), VMValue::Int(timestamp));
        // Pop the closure
        self.pop()?;
        // Push result
        self.push(VMValue::Instance(Rc::new(RefCell::new(instance))));
        Ok(())
    }

    /// Handle duration_between(dt1, dt2) static method call
    fn handle_duration_between(&mut self, arg_count: usize) -> VMResult<()> {
        if arg_count != 2 {
            return Err(RuntimeError::new(
                "duration_between expects 2 arguments",
                Span::default(),
            ));
        }
        let arg2 = self.pop()?;
        let arg1 = self.pop()?;
        let (start, end) = match (arg1, arg2) {
            (VMValue::Instance(i1), VMValue::Instance(i2)) => {
                let ts1 = i1.borrow().get("timestamp");
                let ts2 = i2.borrow().get("timestamp");
                match (ts1, ts2) {
                    (Some(VMValue::Int(t1)), Some(VMValue::Int(t2))) => (t1, t2),
                    _ => {
                        return Err(RuntimeError::new(
                            "duration_between requires DateTime instances",
                            Span::default(),
                        ))
                    }
                }
            }
            _ => {
                return Err(RuntimeError::new(
                    "duration_between requires DateTime instances",
                    Span::default(),
                ))
            }
        };
        let class = Rc::new(RefCell::new(VMClass::new("Duration".to_string())));
        let instance =
            VMInstance::with_field(class, "seconds".to_string(), VMValue::Int(end - start));
        // Pop the closure
        self.pop()?;
        // Push result
        self.push(VMValue::Instance(Rc::new(RefCell::new(instance))));
        Ok(())
    }

    /// Handle duration_*(value) static method calls
    fn handle_duration_from_value(&mut self, method_name: &str, arg_count: usize) -> VMResult<()> {
        if arg_count != 1 {
            return Err(RuntimeError::new(
                format!("duration_{} expects 1 argument", method_name),
                Span::default(),
            ));
        }
        let arg = self.pop()?;
        let seconds = match arg {
            VMValue::Int(n) => n as f64,
            VMValue::Float(n) => n,
            _ => {
                return Err(RuntimeError::new(
                    format!("duration_{} requires a number", method_name),
                    Span::default(),
                ))
            }
        };
        let total_seconds = match method_name {
            "seconds" => seconds,
            "minutes" => seconds * 60.0,
            "hours" => seconds * 3600.0,
            "days" => seconds * 86400.0,
            "weeks" => seconds * 604800.0,
            _ => {
                return Err(RuntimeError::new(
                    format!("Unknown duration method: {}", method_name),
                    Span::default(),
                ))
            }
        };
        let class = Rc::new(RefCell::new(VMClass::new("Duration".to_string())));
        let instance =
            VMInstance::with_field(class, "seconds".to_string(), VMValue::Float(total_seconds));
        // Pop the closure
        self.pop()?;
        // Push result
        self.push(VMValue::Instance(Rc::new(RefCell::new(instance))));
        Ok(())
    }

    /// Handle I18n static method calls
    fn handle_i18n_method(&mut self, function_name: &str, arg_count: usize) -> VMResult<()> {
        match function_name {
            "i18n_locale" => {
                // Pop closure
                self.pop()?;
                // Return current locale
                let locale = vm_get_locale();
                self.push(VMValue::String(Rc::new(locale)));
                Ok(())
            }
            "i18n_set_locale" => {
                if arg_count != 1 {
                    return Err(RuntimeError::new(
                        format!("I18n.set_locale expects 1 argument, got {}", arg_count),
                        Span::default(),
                    ));
                }
                // Collect arguments like other i18n methods
                let mut args = Vec::new();
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();

                let locale = match args.get(0) {
                    Some(VMValue::String(s)) => s.as_str().to_string(),
                    Some(other) => {
                        return Err(RuntimeError::new(
                            format!("I18n.set_locale expects a string, got {}", other.type_name()),
                            Span::default(),
                        ))
                    }
                    None => {
                        return Err(RuntimeError::new(
                            "I18n.set_locale expects a string argument",
                            Span::default(),
                        ))
                    }
                };
                vm_set_locale(locale.clone());
                // Pop closure
                self.pop()?;
                self.push(VMValue::String(Rc::new(locale)));
                Ok(())
            }
            "i18n_translate" => {
                // i18n_translate(key, locale?, translations?)
                // Collect args in reverse order
                let mut args = Vec::new();
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();

                let key = match args.get(0) {
                    Some(VMValue::String(s)) => s.as_str().to_string(),
                    _ => {
                        return Err(RuntimeError::new(
                            "I18n.translate expects a key string",
                            Span::default(),
                        ))
                    }
                };

                let locale = if arg_count > 1 {
                    match &args[1] {
                        VMValue::String(s) => s.as_str().to_string(),
                        VMValue::Null => vm_get_locale(),
                        _ => {
                            return Err(RuntimeError::new(
                                "I18n.translate locale must be a string or null",
                                Span::default(),
                            ))
                        }
                    }
                } else {
                    vm_get_locale()
                };

                let result = if arg_count > 2 {
                    match &args[2] {
                        VMValue::Hash(h) => {
                            let locale_key = format!("{}.{}", locale, key);
                            let fallback_key = format!("en.{}", key);
                            let hash = h.borrow();
                            let found = hash
                                .iter()
                                .find(|(k, _)| {
                                    if let VMValue::String(s) = k {
                                        s.as_str() == locale_key
                                    } else {
                                        false
                                    }
                                })
                                .or_else(|| {
                                    hash.iter().find(|(k, _)| {
                                        if let VMValue::String(s) = k {
                                            s.as_str() == fallback_key
                                        } else {
                                            false
                                        }
                                    })
                                });
                            match found {
                                Some((_, v)) => v.clone(),
                                None => VMValue::String(Rc::new(key)),
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "I18n.translate translations must be a Hash",
                                Span::default(),
                            ))
                        }
                    }
                } else {
                    VMValue::String(Rc::new(key))
                };

                // Pop closure
                self.pop()?;
                self.push(result);
                Ok(())
            }
            "i18n_plural" => {
                // i18n_plural(key, n, locale?, translations?)
                let mut args = Vec::new();
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();

                let key = match args.get(0) {
                    Some(VMValue::String(s)) => s.as_str().to_string(),
                    _ => {
                        return Err(RuntimeError::new(
                            "I18n.plural expects a key string",
                            Span::default(),
                        ))
                    }
                };

                let n = match args.get(1) {
                    Some(VMValue::Int(i)) => *i,
                    Some(VMValue::Float(f)) => *f as i64,
                    _ => {
                        return Err(RuntimeError::new(
                            "I18n.plural expects a number",
                            Span::default(),
                        ))
                    }
                };

                let locale = if arg_count > 2 {
                    match &args[2] {
                        VMValue::String(s) => s.as_str().to_string(),
                        VMValue::Null => vm_get_locale(),
                        _ => {
                            return Err(RuntimeError::new(
                                "I18n.plural locale must be a string or null",
                                Span::default(),
                            ))
                        }
                    }
                } else {
                    vm_get_locale()
                };

                let plural_suffix = if n == 0 {
                    "_zero"
                } else if n == 1 {
                    "_one"
                } else {
                    "_other"
                };
                let plural_key = format!("{}.{}{}", locale, key, plural_suffix);

                let result = if arg_count > 3 {
                    match &args[3] {
                        VMValue::Hash(h) => {
                            let hash = h.borrow();
                            let found = hash.iter().find(|(k, _)| {
                                if let VMValue::String(s) = k {
                                    s.as_str() == plural_key
                                } else {
                                    false
                                }
                            });
                            match found {
                                Some((_, v)) => v.clone(),
                                None => VMValue::String(Rc::new(key)),
                            }
                        }
                        _ => {
                            return Err(RuntimeError::new(
                                "I18n.plural translations must be a Hash",
                                Span::default(),
                            ))
                        }
                    }
                } else {
                    VMValue::String(Rc::new(key))
                };

                // Pop closure
                self.pop()?;
                self.push(result);
                Ok(())
            }
            "i18n_format_number" => {
                // i18n_format_number(n, locale?)
                let mut args = Vec::new();
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();

                let n = match args.get(0) {
                    Some(VMValue::Int(i)) => *i as f64,
                    Some(VMValue::Float(f)) => *f,
                    _ => {
                        return Err(RuntimeError::new(
                            "I18n.format_number expects a number",
                            Span::default(),
                        ))
                    }
                };

                let locale = if arg_count > 1 {
                    match &args[1] {
                        VMValue::String(s) => s.as_str().to_string(),
                        VMValue::Null => vm_get_locale(),
                        _ => {
                            return Err(RuntimeError::new(
                                "I18n.format_number locale must be a string or null",
                                Span::default(),
                            ))
                        }
                    }
                } else {
                    vm_get_locale()
                };

                let formatted = match locale.as_str() {
                    "fr" | "de" | "es" | "it" => format!("{}", n).replace('.', ","),
                    _ => format!("{}", n),
                };

                // Pop closure
                self.pop()?;
                self.push(VMValue::String(Rc::new(formatted)));
                Ok(())
            }
            "i18n_format_currency" => {
                // i18n_format_currency(amount, currency, locale?)
                let mut args = Vec::new();
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();

                let amount = match args.get(0) {
                    Some(VMValue::Int(i)) => *i as f64,
                    Some(VMValue::Float(f)) => *f,
                    _ => {
                        return Err(RuntimeError::new(
                            "I18n.format_currency expects a number",
                            Span::default(),
                        ))
                    }
                };

                let currency = match args.get(1) {
                    Some(VMValue::String(s)) => s.as_str().to_string(),
                    _ => {
                        return Err(RuntimeError::new(
                            "I18n.format_currency expects a currency code",
                            Span::default(),
                        ))
                    }
                };

                let locale = if arg_count > 2 {
                    match &args[2] {
                        VMValue::String(s) => s.as_str().to_string(),
                        VMValue::Null => vm_get_locale(),
                        _ => {
                            return Err(RuntimeError::new(
                                "I18n.format_currency locale must be a string or null",
                                Span::default(),
                            ))
                        }
                    }
                } else {
                    vm_get_locale()
                };

                let symbol = match currency.as_str() {
                    "USD" => "$",
                    "EUR" => "€",
                    "GBP" => "£",
                    "JPY" => "¥",
                    _ => &currency,
                };

                let (decimal_sep, thousands_sep) = match locale.as_str() {
                    "fr" | "de" | "es" | "it" => (",", "."),
                    _ => (".", ","),
                };

                let int_part = amount as i64;
                let frac_part = ((amount - int_part as f64) * 100.0).round() as i64;
                let int_str = int_part.to_string();
                let formatted_int: String = int_str
                    .chars()
                    .rev()
                    .collect::<Vec<_>>()
                    .chunks(3)
                    .map(|chunk| chunk.iter().collect::<String>())
                    .collect::<Vec<_>>()
                    .join(thousands_sep)
                    .chars()
                    .rev()
                    .collect();

                let result = if frac_part > 0 {
                    format!(
                        "{}{}{}{}",
                        symbol,
                        formatted_int,
                        decimal_sep,
                        format!("{:02}", frac_part)
                    )
                } else {
                    format!("{}{}", symbol, formatted_int)
                };

                // Pop closure
                self.pop()?;
                self.push(VMValue::String(Rc::new(result)));
                Ok(())
            }
            "i18n_format_date" => {
                // i18n_format_date(ts, locale?)
                let mut args = Vec::new();
                for _ in 0..arg_count {
                    args.push(self.pop()?);
                }
                args.reverse();

                let ts = match args.get(0) {
                    Some(VMValue::Int(n)) => *n,
                    _ => {
                        return Err(RuntimeError::new(
                            "I18n.format_date requires a timestamp",
                            Span::default(),
                        ))
                    }
                };

                let locale = if arg_count > 1 {
                    match &args[1] {
                        VMValue::String(s) => s.as_str().to_string(),
                        VMValue::Null => vm_get_locale(),
                        _ => {
                            return Err(RuntimeError::new(
                                "I18n.format_date locale must be a string or null",
                                Span::default(),
                            ))
                        }
                    }
                } else {
                    vm_get_locale()
                };

                let dt = chrono::DateTime::from_timestamp(ts, 0)
                    .ok_or_else(|| RuntimeError::new("Invalid timestamp", Span::default()))?;
                let local = dt.with_timezone(&chrono::Local);

                let formatted = match locale.as_str() {
                    "fr" => format!(
                        "{:02}/{:02}/{:04}",
                        local.day(),
                        local.month(),
                        local.year()
                    ),
                    "en" => format!(
                        "{:02}/{:02}/{:04}",
                        local.month(),
                        local.day(),
                        local.year()
                    ),
                    "de" => format!(
                        "{:02}.{:02}.{:04}",
                        local.day(),
                        local.month(),
                        local.year()
                    ),
                    _ => format!(
                        "{:04}-{:02}-{:02}",
                        local.year(),
                        local.month(),
                        local.day()
                    ),
                };

                // Pop closure
                self.pop()?;
                self.push(VMValue::String(Rc::new(formatted)));
                Ok(())
            }
            _ => Err(RuntimeError::new(
                format!("Unknown I18n method: {}", function_name),
                Span::default(),
            )),
        }
    }

    /// Call a closure and return its result (for native function callbacks).
    fn call_closure(
        &mut self,
        closure: Rc<RefCell<Closure>>,
        args: Vec<VMValue>,
    ) -> VMResult<VMValue> {
        let arity = closure.borrow().function.arity as usize;
        if args.len() != arity {
            return Err(RuntimeError::new(
                format!("Expected {} arguments but got {}", arity, args.len()),
                Span::default(),
            ));
        }

        // Push arguments
        for arg in args {
            self.push(arg);
        }

        let slots_start = self.stack.len() - arity;

        self.frames.push(CallFrame {
            closure,
            ip: 0,
            slots_start,
        });

        // Execute until this frame returns
        self.execute()?;

        // The result should be on top of stack
        self.pop()
    }

    fn invoke(&mut self, name: &str, arg_count: usize) -> VMResult<()> {
        let receiver = self.peek(arg_count)?.clone();

        if let VMValue::Instance(instance) = receiver {
            // Check for field first
            if let Some(value) = instance.borrow().get(name) {
                // It's a field that happens to be callable
                let callee_idx = self.stack.len() - arg_count - 1;
                self.stack[callee_idx] = value;
                return self.call_value(arg_count);
            }

            // Look up method
            let class = instance.borrow().class.clone();
            if let Some(method) = class.borrow().find_method(name) {
                return self.call_closure_frame(method, arg_count + 1);
            }

            Err(RuntimeError::new(
                format!("Undefined property '{}'", name),
                Span::default(),
            ))
        } else {
            Err(RuntimeError::new(
                format!("Only instances have methods, got {}", receiver.type_name()),
                Span::default(),
            ))
        }
    }

    fn define_method(&mut self, name: &str, is_static: bool) -> VMResult<()> {
        let method = self.pop()?;
        let class = self.peek(0)?.clone();

        if let (VMValue::Closure(method), VMValue::Class(class)) = (method, class) {
            if is_static {
                class
                    .borrow_mut()
                    .static_methods
                    .insert(name.to_string(), method);
            } else if name == "constructor" {
                class.borrow_mut().constructor = Some(method);
            } else {
                class.borrow_mut().methods.insert(name.to_string(), method);
            }
            Ok(())
        } else {
            Err(RuntimeError::new(
                "Invalid method definition",
                Span::default(),
            ))
        }
    }

    fn capture_upvalue(&mut self, slot: usize) -> Rc<RefCell<Upvalue>> {
        // Check if we already have an open upvalue for this slot
        for upvalue in &self.open_upvalues {
            if let Upvalue::Open(s) = &*upvalue.borrow() {
                if *s == slot {
                    return upvalue.clone();
                }
            }
        }

        // Create new upvalue
        let upvalue = Rc::new(RefCell::new(Upvalue::Open(slot)));
        self.open_upvalues.push(upvalue.clone());
        upvalue
    }

    fn close_upvalues(&mut self, last_slot: usize) {
        // Close all upvalues pointing to slots >= last_slot
        for upvalue in &self.open_upvalues {
            let mut uv = upvalue.borrow_mut();
            if let Upvalue::Open(slot) = &*uv {
                if *slot >= last_slot {
                    let value = self.stack[*slot].clone();
                    *uv = Upvalue::Closed(value);
                }
            }
        }

        // Remove closed upvalues from the list
        self.open_upvalues.retain(|uv| uv.borrow().is_open());
    }
}

impl Default for VM {
    fn default() -> Self {
        Self::new()
    }
}

// ========== JSON Helper Functions ==========

/// Convert a VMValue to a JSON string.
fn vm_value_to_json(value: &VMValue) -> Result<String, RuntimeError> {
    let json = vm_value_to_serde_json(value)?;
    serde_json::to_string(&json)
        .map_err(|e| RuntimeError::new(format!("JSON serialization error: {}", e), Span::default()))
}

/// Extract status code from a response hash or integer.
fn extract_vm_status(value: &VMValue) -> Result<i64, RuntimeError> {
    match value {
        VMValue::Int(n) => Ok(*n),
        VMValue::Hash(hash) => {
            for (k, v) in hash.borrow().iter() {
                if let VMValue::String(key) = k {
                    if key.as_str() == "status" {
                        if let VMValue::Int(status) = v {
                            return Ok(*status);
                        }
                    }
                }
            }
            Err(RuntimeError::new(
                "Response hash does not contain 'status' field".to_string(),
                Span::default(),
            ))
        }
        other => Err(RuntimeError::new(
            format!(
                "Expected response hash or status code, got {}",
                other.type_name()
            ),
            Span::default(),
        )),
    }
}

/// Convert a VMValue to serde_json::Value.
fn vm_value_to_serde_json(value: &VMValue) -> Result<serde_json::Value, RuntimeError> {
    match value {
        VMValue::Null => Ok(serde_json::Value::Null),
        VMValue::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        VMValue::Int(n) => Ok(serde_json::Value::Number((*n).into())),
        VMValue::Float(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                RuntimeError::new(
                    "Cannot convert float to JSON (NaN or Infinity)".to_string(),
                    Span::default(),
                )
            }),
        VMValue::String(s) => Ok(serde_json::Value::String((**s).clone())),
        VMValue::Array(arr) => {
            let items: Result<Vec<serde_json::Value>, RuntimeError> =
                arr.borrow().iter().map(vm_value_to_serde_json).collect();
            Ok(serde_json::Value::Array(items?))
        }
        VMValue::Hash(hash) => {
            let mut map = serde_json::Map::new();
            for (k, v) in hash.borrow().iter() {
                let key = match k {
                    VMValue::String(s) => (**s).clone(),
                    _ => format!("{}", k),
                };
                map.insert(key, vm_value_to_serde_json(v)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        other => Err(RuntimeError::new(
            format!("Cannot convert {} to JSON", other.type_name()),
            Span::default(),
        )),
    }
}

/// Convert a serde_json::Value to a VMValue.
fn json_to_vm_value(json: &serde_json::Value) -> Result<VMValue, RuntimeError> {
    match json {
        serde_json::Value::Null => Ok(VMValue::Null),
        serde_json::Value::Bool(b) => Ok(VMValue::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(VMValue::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(VMValue::Float(f))
            } else {
                Err(RuntimeError::new(
                    "Invalid JSON number".to_string(),
                    Span::default(),
                ))
            }
        }
        serde_json::Value::String(s) => Ok(VMValue::String(Rc::new(s.clone()))),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<VMValue>, RuntimeError> =
                arr.iter().map(json_to_vm_value).collect();
            Ok(VMValue::Array(Rc::new(RefCell::new(items?))))
        }
        serde_json::Value::Object(obj) => {
            let pairs: Result<Vec<(VMValue, VMValue)>, RuntimeError> = obj
                .iter()
                .map(|(k, v)| Ok((VMValue::String(Rc::new(k.clone())), json_to_vm_value(v)?)))
                .collect();
            Ok(VMValue::Hash(Rc::new(RefCell::new(pairs?))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::compiler::Compiler;

    fn run_source(source: &str) -> VMResult<()> {
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .map_err(|e| RuntimeError::new(e.to_string(), Span::default()))?;
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| RuntimeError::new(e.to_string(), Span::default()))?;

        let mut compiler = Compiler::new();
        let function = compiler
            .compile(&program)
            .map_err(|e| RuntimeError::new(e.to_string(), Span::default()))?;

        let mut vm = VM::new();
        vm.run(function)
    }

    #[test]
    fn test_simple_arithmetic() {
        assert!(run_source("let x = 1 + 2;").is_ok());
    }

    #[test]
    fn test_variables() {
        assert!(run_source("let x = 42; let y = x + 1;").is_ok());
    }

    #[test]
    fn test_function_call() {
        let source = r#"
            fn add(a: Int, b: Int) -> Int {
                return a + b;
            }
            let result = add(1, 2);
        "#;
        assert!(run_source(source).is_ok());
    }

    #[test]
    fn test_while_loop() {
        let source = r#"
            let x = 0;
            while (x < 5) {
                x = x + 1;
            }
        "#;
        assert!(run_source(source).is_ok());
    }
}
