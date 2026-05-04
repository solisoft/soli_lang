//! Image built-in class for SoliLang.
//!
//! Provides the Image class for image manipulation:
//! - Image.new(path) - Load image from file path
//! - Image.from_buffer(base64_string) - Load image from base64-encoded data
//!
//! Instance methods:
//! - img.width, img.height - Get dimensions
//! - img.resize(w, h) - Resize image
//! - img.thumbnail(size) - Create thumbnail (preserves aspect ratio)
//! - img.crop(x, y, w, h) - Crop region
//! - img.quality(n) - Set output quality (1-100)
//! - img.format(fmt) - Set output format
//! - img.grayscale() - Convert to grayscale
//! - img.flip_horizontal() - Flip horizontally
//! - img.flip_vertical() - Flip vertically
//! - img.rotate90(), rotate180(), rotate270() - Rotate
//! - img.blur(sigma) - Gaussian blur
//! - img.brightness(n) - Adjust brightness
//! - img.contrast(n) - Adjust contrast
//! - img.invert() - Invert colors
//! - img.hue_rotate(degrees) - Rotate hue
//! - img.to_buffer() - Get base64-encoded bytes
//! - img.to_file(path) - Save to file

use image::{DynamicImage, ImageFormat, ImageReader};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::rc::Rc;
use std::thread;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, Instance, NativeFunction, Value};

#[derive(Clone, Debug)]
pub struct ImageData {
    pub image: DynamicImage,
    pub format: Option<ImageFormat>,
    pub quality: u8,
}

fn format_from_str(s: &str) -> Option<ImageFormat> {
    match s.to_lowercase().as_str() {
        "jpeg" | "jpg" => Some(ImageFormat::Jpeg),
        "png" => Some(ImageFormat::Png),
        "gif" => Some(ImageFormat::Gif),
        "bmp" => Some(ImageFormat::Bmp),
        "ico" => Some(ImageFormat::Ico),
        "tiff" | "tif" => Some(ImageFormat::Tiff),
        "webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

fn get_image_class() -> Rc<Class> {
    thread_local! {
        static CLASS: Rc<Class> = build_image_class();
    }
    CLASS.with(|c| c.clone())
}

fn image_data_to_value(data: ImageData) -> Value {
    let class = get_image_class();
    let mut inst = Instance::new(class);
    inst.set(
        "__image_data".to_string(),
        Value::Image(Rc::new(RefCell::new(data))),
    );
    Value::Instance(Rc::new(RefCell::new(inst)))
}

fn with_image_data<F, R>(args: &[Value], f: F) -> Result<R, String>
where
    F: FnOnce(&ImageData) -> Result<R, String>,
{
    let this = match args.first() {
        Some(Value::Instance(i)) => i,
        _ => return Err("requires Image instance".to_string()),
    };
    let field = this
        .borrow()
        .get("__image_data")
        .ok_or("Missing image data")?;
    match field {
        Value::Image(img) => f(&img.borrow()),
        _ => Err("Invalid image data".to_string()),
    }
}

fn transform_image<F>(args: &[Value], f: F) -> Result<Value, String>
where
    F: FnOnce(&ImageData) -> Result<DynamicImage, String>,
{
    with_image_data(args, |data| {
        let new_image = f(data)?;
        Ok(image_data_to_value(ImageData {
            image: new_image,
            format: data.format,
            quality: data.quality,
        }))
    })
}

fn encode_dynamic_image(
    img: &DynamicImage,
    quality: u8,
    format: ImageFormat,
) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    let cursor = Cursor::new(&mut buffer);
    if format == ImageFormat::Jpeg {
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(cursor, quality);
        img.write_with_encoder(encoder)
            .map_err(|e| format!("Failed to encode JPEG: {}", e))?;
    } else {
        img.write_to(cursor, format)
            .map_err(|e| format!("Failed to encode image: {}", e))?;
    }
    Ok(buffer)
}

fn encode_image(data: &ImageData, format: ImageFormat) -> Result<Vec<u8>, String> {
    encode_dynamic_image(&data.image, data.quality, format)
}

#[derive(Clone, Debug)]
pub enum PlanOp {
    Resize(u32, u32),
    Thumbnail(u32),
    Crop(u32, u32, u32, u32),
    Grayscale,
    FlipHorizontal,
    FlipVertical,
    Rotate90,
    Rotate180,
    Rotate270,
    Blur(f32),
    Brightness(i32),
    Contrast(f32),
    Invert,
    HueRotate(i32),
}

#[derive(Clone, Debug)]
pub struct ImagePlan {
    pub src: String,
    pub ops: Vec<PlanOp>,
    pub format: Option<ImageFormat>,
    pub quality: u8,
    pub dst: Option<String>,
}

enum PlanResult {
    Saved,
    Image(ImageData),
}

fn apply_plan_op(img: DynamicImage, op: &PlanOp) -> DynamicImage {
    use image::imageops;
    match op {
        PlanOp::Resize(w, h) => img.resize(*w, *h, imageops::FilterType::Lanczos3),
        PlanOp::Thumbnail(s) => img.thumbnail(*s, *s),
        PlanOp::Crop(x, y, w, h) => {
            DynamicImage::ImageRgba8(imageops::crop_imm(&img, *x, *y, *w, *h).to_image())
        }
        PlanOp::Grayscale => img.grayscale(),
        PlanOp::FlipHorizontal => DynamicImage::ImageRgba8(imageops::flip_horizontal(&img)),
        PlanOp::FlipVertical => DynamicImage::ImageRgba8(imageops::flip_vertical(&img)),
        PlanOp::Rotate90 => img.rotate90(),
        PlanOp::Rotate180 => img.rotate180(),
        PlanOp::Rotate270 => img.rotate270(),
        PlanOp::Blur(sigma) => DynamicImage::ImageRgba8(imageops::blur(&img, *sigma)),
        PlanOp::Brightness(v) => DynamicImage::ImageRgba8(imageops::brighten(&img, *v)),
        PlanOp::Contrast(v) => DynamicImage::ImageRgba8(imageops::contrast(&img, *v)),
        PlanOp::Invert => {
            let mut i = img;
            imageops::invert(&mut i);
            i
        }
        PlanOp::HueRotate(d) => DynamicImage::ImageRgba8(imageops::huerotate(&img, *d)),
    }
}

fn execute_plan(plan: &ImagePlan) -> Result<PlanResult, String> {
    let reader =
        ImageReader::open(&plan.src).map_err(|e| format!("Failed to open image: {}", e))?;
    let detected_format = reader.format();
    let mut img = reader
        .decode()
        .map_err(|e| format!("Failed to decode image: {}", e))?;
    for op in &plan.ops {
        img = apply_plan_op(img, op);
    }
    let final_format = plan.format.or(detected_format);
    if let Some(dst) = &plan.dst {
        let format =
            final_format.unwrap_or_else(|| ImageFormat::from_path(dst).unwrap_or(ImageFormat::Png));
        if format == ImageFormat::Jpeg {
            let buffer = encode_dynamic_image(&img, plan.quality, format)?;
            std::fs::write(dst, buffer).map_err(|e| format!("Failed to write file: {}", e))?;
        } else {
            img.save(dst)
                .map_err(|e| format!("Failed to save image: {}", e))?;
        }
        Ok(PlanResult::Saved)
    } else {
        Ok(PlanResult::Image(ImageData {
            image: img,
            format: final_format,
            quality: plan.quality,
        }))
    }
}

fn get_image_plan_class() -> Rc<Class> {
    thread_local! {
        static CLASS: Rc<Class> = build_image_plan_class();
    }
    CLASS.with(|c| c.clone())
}

fn plan_to_value(plan: ImagePlan) -> Value {
    let class = get_image_plan_class();
    let mut inst = Instance::new(class);
    inst.set(
        "__image_plan".to_string(),
        Value::ImagePlan(Rc::new(RefCell::new(plan))),
    );
    Value::Instance(Rc::new(RefCell::new(inst)))
}

fn with_plan<F, R>(args: &[Value], f: F) -> Result<R, String>
where
    F: FnOnce(&ImagePlan) -> Result<R, String>,
{
    let this = match args.first() {
        Some(Value::Instance(i)) => i,
        _ => return Err("requires ImagePlan instance".to_string()),
    };
    let field = this
        .borrow()
        .get("__image_plan")
        .ok_or("Missing image plan")?;
    match field {
        Value::ImagePlan(p) => f(&p.borrow()),
        _ => Err("Invalid plan data".to_string()),
    }
}

fn extend_plan<F>(args: &[Value], f: F) -> Result<Value, String>
where
    F: FnOnce(&mut ImagePlan),
{
    let new_plan = with_plan(args, |plan| {
        let mut np = plan.clone();
        f(&mut np);
        Ok(np)
    })?;
    Ok(plan_to_value(new_plan))
}

fn record_op(args: &[Value], op: PlanOp) -> Result<Value, String> {
    extend_plan(args, |p| p.ops.push(op))
}

fn extract_plan(value: &Value) -> Result<ImagePlan, String> {
    let inst = match value {
        Value::Instance(i) => i,
        other => {
            return Err(format!(
                "expected ImagePlan instance, got {}",
                other.type_name()
            ))
        }
    };
    let field = inst.borrow().get("__image_plan").ok_or_else(|| {
        format!(
            "expected ImagePlan instance (class {})",
            inst.borrow().class.name
        )
    })?;
    match field {
        Value::ImagePlan(p) => Ok(p.borrow().clone()),
        _ => Err("Invalid plan data".to_string()),
    }
}

pub fn register_image_class(env: &mut Environment) {
    let class = get_image_class();
    env.define("Image".to_string(), Value::Class(class));
}

fn build_image_class() -> Rc<Class> {
    let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    native_methods.insert(
        "width".to_string(),
        Rc::new(NativeFunction::new("Image.width", Some(0), |args| {
            with_image_data(&args, |data| Ok(Value::Int(data.image.width() as i64)))
        })),
    );

    native_methods.insert(
        "height".to_string(),
        Rc::new(NativeFunction::new("Image.height", Some(0), |args| {
            with_image_data(&args, |data| Ok(Value::Int(data.image.height() as i64)))
        })),
    );

    native_methods.insert(
        "resize".to_string(),
        Rc::new(NativeFunction::new("Image.resize", Some(2), |args| {
            let width = match &args[1] {
                Value::Int(n) => *n as u32,
                _ => return Err("Image.resize requires integer width".to_string()),
            };
            let height = match &args[2] {
                Value::Int(n) => *n as u32,
                _ => return Err("Image.resize requires integer height".to_string()),
            };
            transform_image(&args, |data| {
                Ok(data
                    .image
                    .resize(width, height, image::imageops::FilterType::Lanczos3))
            })
        })),
    );

    native_methods.insert(
        "thumbnail".to_string(),
        Rc::new(NativeFunction::new("Image.thumbnail", Some(1), |args| {
            let max_size = match &args[1] {
                Value::Int(n) => *n as u32,
                _ => return Err("Image.thumbnail requires integer size".to_string()),
            };
            transform_image(&args, |data| Ok(data.image.thumbnail(max_size, max_size)))
        })),
    );

    native_methods.insert(
        "crop".to_string(),
        Rc::new(NativeFunction::new("Image.crop", Some(4), |args| {
            let x = match &args[1] {
                Value::Int(n) if *n >= 0 => *n as u32,
                Value::Int(_) => return Err("Image.crop requires non-negative x".to_string()),
                _ => return Err("Image.crop requires integer x".to_string()),
            };
            let y = match &args[2] {
                Value::Int(n) if *n >= 0 => *n as u32,
                Value::Int(_) => return Err("Image.crop requires non-negative y".to_string()),
                _ => return Err("Image.crop requires integer y".to_string()),
            };
            let width = match &args[3] {
                Value::Int(n) => *n as u32,
                _ => return Err("Image.crop requires integer width".to_string()),
            };
            let height = match &args[4] {
                Value::Int(n) => *n as u32,
                _ => return Err("Image.crop requires integer height".to_string()),
            };
            transform_image(&args, |data| {
                let cropped =
                    image::imageops::crop_imm(&data.image, x, y, width, height).to_image();
                Ok(DynamicImage::ImageRgba8(cropped))
            })
        })),
    );

    native_methods.insert(
        "quality".to_string(),
        Rc::new(NativeFunction::new("Image.quality", Some(1), |args| {
            let quality = match &args[1] {
                Value::Int(n) => (*n).clamp(1, 100) as u8,
                _ => return Err("Image.quality requires integer".to_string()),
            };
            with_image_data(&args, |data| {
                Ok(image_data_to_value(ImageData {
                    image: data.image.clone(),
                    format: data.format,
                    quality,
                }))
            })
        })),
    );

    native_methods.insert(
        "format".to_string(),
        Rc::new(NativeFunction::new("Image.format", Some(1), |args| {
            let fmt = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err("Image.format requires string".to_string()),
            };
            let format =
                format_from_str(&fmt).ok_or_else(|| format!("Unsupported format: {}", fmt))?;
            with_image_data(&args, |data| {
                Ok(image_data_to_value(ImageData {
                    image: data.image.clone(),
                    format: Some(format),
                    quality: data.quality,
                }))
            })
        })),
    );

    native_methods.insert(
        "grayscale".to_string(),
        Rc::new(NativeFunction::new("Image.grayscale", Some(0), |args| {
            transform_image(&args, |data| Ok(data.image.grayscale()))
        })),
    );

    native_methods.insert(
        "flip_horizontal".to_string(),
        Rc::new(NativeFunction::new(
            "Image.flip_horizontal",
            Some(0),
            |args| {
                transform_image(&args, |data| {
                    Ok(DynamicImage::ImageRgba8(image::imageops::flip_horizontal(
                        &data.image,
                    )))
                })
            },
        )),
    );

    native_methods.insert(
        "flip_vertical".to_string(),
        Rc::new(NativeFunction::new(
            "Image.flip_vertical",
            Some(0),
            |args| {
                transform_image(&args, |data| {
                    Ok(DynamicImage::ImageRgba8(image::imageops::flip_vertical(
                        &data.image,
                    )))
                })
            },
        )),
    );

    native_methods.insert(
        "rotate90".to_string(),
        Rc::new(NativeFunction::new("Image.rotate90", Some(0), |args| {
            transform_image(&args, |data| Ok(data.image.rotate90()))
        })),
    );

    native_methods.insert(
        "rotate180".to_string(),
        Rc::new(NativeFunction::new("Image.rotate180", Some(0), |args| {
            transform_image(&args, |data| Ok(data.image.rotate180()))
        })),
    );

    native_methods.insert(
        "rotate270".to_string(),
        Rc::new(NativeFunction::new("Image.rotate270", Some(0), |args| {
            transform_image(&args, |data| Ok(data.image.rotate270()))
        })),
    );

    native_methods.insert(
        "blur".to_string(),
        Rc::new(NativeFunction::new("Image.blur", Some(1), |args| {
            let sigma = match &args[1] {
                Value::Float(f) => *f as f32,
                Value::Int(n) => *n as f32,
                _ => return Err("Image.blur requires number".to_string()),
            };
            transform_image(&args, |data| {
                Ok(DynamicImage::ImageRgba8(image::imageops::blur(
                    &data.image,
                    sigma,
                )))
            })
        })),
    );

    native_methods.insert(
        "brightness".to_string(),
        Rc::new(NativeFunction::new("Image.brightness", Some(1), |args| {
            let value = match &args[1] {
                Value::Int(n) => *n as i32,
                _ => return Err("Image.brightness requires integer".to_string()),
            };
            transform_image(&args, |data| {
                Ok(DynamicImage::ImageRgba8(image::imageops::brighten(
                    &data.image,
                    value,
                )))
            })
        })),
    );

    native_methods.insert(
        "contrast".to_string(),
        Rc::new(NativeFunction::new("Image.contrast", Some(1), |args| {
            let value = match &args[1] {
                Value::Float(f) => *f as f32,
                Value::Int(n) => *n as f32,
                _ => return Err("Image.contrast requires number".to_string()),
            };
            transform_image(&args, |data| {
                Ok(DynamicImage::ImageRgba8(image::imageops::contrast(
                    &data.image,
                    value,
                )))
            })
        })),
    );

    native_methods.insert(
        "invert".to_string(),
        Rc::new(NativeFunction::new("Image.invert", Some(0), |args| {
            transform_image(&args, |data| {
                let mut inverted = data.image.clone();
                image::imageops::invert(&mut inverted);
                Ok(inverted)
            })
        })),
    );

    native_methods.insert(
        "hue_rotate".to_string(),
        Rc::new(NativeFunction::new("Image.hue_rotate", Some(1), |args| {
            let degrees = match &args[1] {
                Value::Int(n) => *n as i32,
                _ => return Err("Image.hue_rotate requires integer".to_string()),
            };
            transform_image(&args, |data| {
                Ok(DynamicImage::ImageRgba8(image::imageops::huerotate(
                    &data.image,
                    degrees,
                )))
            })
        })),
    );

    native_methods.insert(
        "to_buffer".to_string(),
        Rc::new(NativeFunction::new("Image.to_buffer", Some(0), |args| {
            with_image_data(&args, |data| {
                let format = data.format.unwrap_or(ImageFormat::Png);
                let buffer = encode_image(data, format)?;
                Ok(Value::String(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &buffer,
                )))
            })
        })),
    );

    native_methods.insert(
        "to_file".to_string(),
        Rc::new(NativeFunction::new("Image.to_file", Some(1), |args| {
            let path = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err("Image.to_file requires string path".to_string()),
            };
            with_image_data(&args, |data| {
                let format = data
                    .format
                    .unwrap_or_else(|| ImageFormat::from_path(&path).unwrap_or(ImageFormat::Png));
                if format == ImageFormat::Jpeg {
                    let buffer = encode_image(data, format)?;
                    std::fs::write(&path, buffer)
                        .map_err(|e| format!("Failed to write file: {}", e))?;
                } else {
                    data.image
                        .save(&path)
                        .map_err(|e| format!("Failed to save image: {}", e))?;
                }
                Ok(Value::Bool(true))
            })
        })),
    );

    // Static methods
    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    static_methods.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("Image.new", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("Image.new requires string path".to_string()),
            };
            let reader =
                ImageReader::open(&path).map_err(|e| format!("Failed to open image: {}", e))?;
            let format = reader.format();
            let img = reader
                .decode()
                .map_err(|e| format!("Failed to decode image: {}", e))?;

            Ok(image_data_to_value(ImageData {
                image: img,
                format,
                quality: 85,
            }))
        })),
    );

    static_methods.insert(
        "plan".to_string(),
        Rc::new(NativeFunction::new("Image.plan", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("Image.plan requires string path".to_string()),
            };
            Ok(plan_to_value(ImagePlan {
                src: path,
                ops: Vec::new(),
                format: None,
                quality: 85,
                dst: None,
            }))
        })),
    );

    static_methods.insert(
        "process_all".to_string(),
        Rc::new(NativeFunction::new("Image.process_all", Some(1), |args| {
            let plans: Vec<ImagePlan> = match &args[0] {
                Value::Array(arr) => {
                    let mut out = Vec::with_capacity(arr.borrow().len());
                    for item in arr.borrow().iter() {
                        out.push(extract_plan(item)?);
                    }
                    out
                }
                other => {
                    return Err(format!(
                        "Image.process_all() expects array of plans, got {}",
                        other.type_name()
                    ))
                }
            };

            let handles: Vec<_> = plans
                .into_iter()
                .map(|plan| thread::spawn(move || execute_plan(&plan)))
                .collect();

            let results: Vec<Value> = handles
                .into_iter()
                .map(|h| match h.join() {
                    Ok(Ok(PlanResult::Saved)) => Value::Bool(true),
                    Ok(Ok(PlanResult::Image(data))) => image_data_to_value(data),
                    Ok(Err(e)) => hash_from_pairs([("error".to_string(), Value::String(e))]),
                    Err(_) => hash_from_pairs([(
                        "error".to_string(),
                        Value::String("Thread panicked".to_string()),
                    )]),
                })
                .collect();

            Ok(Value::Array(Rc::new(RefCell::new(results))))
        })),
    );

    static_methods.insert(
        "from_buffer".to_string(),
        Rc::new(NativeFunction::new("Image.from_buffer", Some(1), |args| {
            let buffer = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("Image.from_buffer requires string buffer".to_string()),
            };
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                buffer.as_bytes(),
            )
            .map_err(|e| format!("Failed to decode base64 buffer: {}", e))?;

            let format = image::guess_format(&bytes).ok();
            let img = image::load_from_memory(&bytes)
                .map_err(|e| format!("Failed to load image from buffer: {}", e))?;

            Ok(image_data_to_value(ImageData {
                image: img,
                format,
                quality: 85,
            }))
        })),
    );

    let image_class = Class {
        name: "Image".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: static_methods,
        native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    Rc::new(image_class)
}

fn build_image_plan_class() -> Rc<Class> {
    let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    fn int_arg(args: &[Value], idx: usize, who: &str) -> Result<i64, String> {
        match args.get(idx) {
            Some(Value::Int(n)) => Ok(*n),
            _ => Err(format!("{} requires integer argument", who)),
        }
    }

    fn nonneg_u32(args: &[Value], idx: usize, who: &str) -> Result<u32, String> {
        match args.get(idx) {
            Some(Value::Int(n)) if *n >= 0 => Ok(*n as u32),
            Some(Value::Int(_)) => Err(format!("{} requires non-negative integer", who)),
            _ => Err(format!("{} requires integer", who)),
        }
    }

    fn float_arg(args: &[Value], idx: usize, who: &str) -> Result<f32, String> {
        match args.get(idx) {
            Some(Value::Float(f)) => Ok(*f as f32),
            Some(Value::Int(n)) => Ok(*n as f32),
            _ => Err(format!("{} requires number", who)),
        }
    }

    native_methods.insert(
        "resize".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.resize", Some(2), |args| {
            let w = nonneg_u32(&args, 1, "ImagePlan.resize")?;
            let h = nonneg_u32(&args, 2, "ImagePlan.resize")?;
            record_op(&args, PlanOp::Resize(w, h))
        })),
    );
    native_methods.insert(
        "thumbnail".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.thumbnail",
            Some(1),
            |args| {
                let s = nonneg_u32(&args, 1, "ImagePlan.thumbnail")?;
                record_op(&args, PlanOp::Thumbnail(s))
            },
        )),
    );
    native_methods.insert(
        "crop".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.crop", Some(4), |args| {
            let x = nonneg_u32(&args, 1, "ImagePlan.crop")?;
            let y = nonneg_u32(&args, 2, "ImagePlan.crop")?;
            let w = nonneg_u32(&args, 3, "ImagePlan.crop")?;
            let h = nonneg_u32(&args, 4, "ImagePlan.crop")?;
            record_op(&args, PlanOp::Crop(x, y, w, h))
        })),
    );
    native_methods.insert(
        "grayscale".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.grayscale",
            Some(0),
            |args| record_op(&args, PlanOp::Grayscale),
        )),
    );
    native_methods.insert(
        "flip_horizontal".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.flip_horizontal",
            Some(0),
            |args| record_op(&args, PlanOp::FlipHorizontal),
        )),
    );
    native_methods.insert(
        "flip_vertical".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.flip_vertical",
            Some(0),
            |args| record_op(&args, PlanOp::FlipVertical),
        )),
    );
    native_methods.insert(
        "rotate90".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.rotate90", Some(0), |args| {
            record_op(&args, PlanOp::Rotate90)
        })),
    );
    native_methods.insert(
        "rotate180".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.rotate180",
            Some(0),
            |args| record_op(&args, PlanOp::Rotate180),
        )),
    );
    native_methods.insert(
        "rotate270".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.rotate270",
            Some(0),
            |args| record_op(&args, PlanOp::Rotate270),
        )),
    );
    native_methods.insert(
        "blur".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.blur", Some(1), |args| {
            let sigma = float_arg(&args, 1, "ImagePlan.blur")?;
            record_op(&args, PlanOp::Blur(sigma))
        })),
    );
    native_methods.insert(
        "brightness".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.brightness",
            Some(1),
            |args| {
                let v = int_arg(&args, 1, "ImagePlan.brightness")? as i32;
                record_op(&args, PlanOp::Brightness(v))
            },
        )),
    );
    native_methods.insert(
        "contrast".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.contrast", Some(1), |args| {
            let v = float_arg(&args, 1, "ImagePlan.contrast")?;
            record_op(&args, PlanOp::Contrast(v))
        })),
    );
    native_methods.insert(
        "invert".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.invert", Some(0), |args| {
            record_op(&args, PlanOp::Invert)
        })),
    );
    native_methods.insert(
        "hue_rotate".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.hue_rotate",
            Some(1),
            |args| {
                let d = int_arg(&args, 1, "ImagePlan.hue_rotate")? as i32;
                record_op(&args, PlanOp::HueRotate(d))
            },
        )),
    );
    native_methods.insert(
        "format".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.format", Some(1), |args| {
            let fmt = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err("ImagePlan.format requires string".to_string()),
            };
            let format =
                format_from_str(&fmt).ok_or_else(|| format!("Unsupported format: {}", fmt))?;
            extend_plan(&args, |p| p.format = Some(format))
        })),
    );
    native_methods.insert(
        "quality".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.quality", Some(1), |args| {
            let q = match &args[1] {
                Value::Int(n) => (*n).clamp(1, 100) as u8,
                _ => return Err("ImagePlan.quality requires integer".to_string()),
            };
            extend_plan(&args, |p| p.quality = q)
        })),
    );
    native_methods.insert(
        "save_to".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.save_to", Some(1), |args| {
            let path = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err("ImagePlan.save_to requires string path".to_string()),
            };
            extend_plan(&args, |p| p.dst = Some(path))
        })),
    );
    native_methods.insert(
        "run".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.run", Some(0), |args| {
            let plan = with_plan(&args, |p| Ok(p.clone()))?;
            match execute_plan(&plan)? {
                PlanResult::Saved => Ok(Value::Bool(true)),
                PlanResult::Image(data) => Ok(image_data_to_value(data)),
            }
        })),
    );
    native_methods.insert(
        "src".to_string(),
        Rc::new(NativeFunction::new("ImagePlan.src", Some(0), |args| {
            with_plan(&args, |p| Ok(Value::String(p.src.clone())))
        })),
    );
    native_methods.insert(
        "ops_count".to_string(),
        Rc::new(NativeFunction::new(
            "ImagePlan.ops_count",
            Some(0),
            |args| with_plan(&args, |p| Ok(Value::Int(p.ops.len() as i64))),
        )),
    );

    let plan_class = Class {
        name: "ImagePlan".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    Rc::new(plan_class)
}
